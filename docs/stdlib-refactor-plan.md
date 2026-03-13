# Stdlib And Template Refactor Plan

## Goal

Move stdlib and area templates out of `adapters/*` as runtime sources and into editable git repos under the `system` namespace.

Target end state:

- `system/stdlib` is the live stdlib repo and area
- each area template lives in its own `system/template_*` repo
- `adapters/*` is only used as a bootstrap source on first startup
- after bootstrap, stdlib and templates are edited through the built-in git/editor flows
- stdlib reloads the same way normal areas reload
- stdlib web definitions must also live in git and reload from there

## Locked Decisions

These decisions are agreed and should be treated as requirements for the refactor:

- there is exactly one active stdlib at runtime
- the active stdlib is selected on first startup only
- first startup provisions the selected stdlib into `system/stdlib`
- after bootstrap, runtime always uses `system/stdlib` as the only stdlib source
- `system/stdlib` is a loadable area
- `system/stdlib` owns the full global portal, including routes, views, helpers, design, and app logic
- the adapter retains only runtime shell responsibilities and must dynamically load portal code from `system/stdlib`
- stdlib reload must happen in-process and should not require adapter restart
- stdlib reload should be diff-aware and distinguish world or command changes, portal or web changes, and base or helper changes
- portal changes should not force unrelated world reloads, and trivial world fixes should not force portal reloads
- when `system/stdlib` changes, the driver is responsible for detecting that and initiating reload
- stdlib changes should be able to trigger reload of dependent loaded areas and objects using the driver's dependency or object overview
- each area template lives in its own `system/template_*` repo
- template repos are not loadable areas for now
- new builder area repos should be created from template repo contents as a fresh repo with a new initial commit
- builder repos must not inherit template git history
- first-start bootstrap config only controls what gets provisioned initially
- after bootstrap, runtime template discovery comes from existing `system/template_*` repos, not config
- `adapters/` should become runtime-only and should not contain live stdlib or live templates
- adapter-driven `set_area_template` support may remain temporarily during migration, but must not remain part of the final runtime model
- bootstrap assets may remain near `adapters/` temporarily, but should later move to a dedicated bootstrap source
- bootstrap config belongs at its own top level, not under `adapters`
- bootstrap config should support both YAML and environment variable overrides
- system repos should only be visible in repo lists and UI to users who actually have access
- bootstrap write access to system repos should default to admin only
- long-term ACL policy for system repos should be definable by stdlib rather than hardcoded permanently in the driver
- each template repo should include a metadata file such as `template.yml`
- first implementation of stdlib-owned ACL should be request-time driver to stdlib RPC, with caching as a later optimization
- template repo `main` is the source branch for creating new builder repos
- a newly created builder repo should get both `main` and `develop`, initially pointing to the same seeded commit
- first implementation of Ruby stdlib reload should be subsystem-based rather than file-by-file constant surgery
- first implementation of stdlib dependency propagation should be subsystem-based rather than a full explicit dependency graph

## Directory Boundary

The `adapters/` directory should be narrowed in responsibility.

Target responsibility for `adapters/`:

- runtime process startup
- MOP client and protocol implementation
- language-specific loader/runtime
- language execution support
- hot-reload machinery for that language

Things that should not live in `adapters/` long-term:

- live stdlib source
- live area template source
- editable game/application content

In other words:

- `adapters/` should contain engine/runtime code
- `system/*` repos should contain editable content
- bootstrap assets may temporarily come from outside `adapters/`, but should not be treated as runtime source of truth

## Proposed System Repos

Create one system stdlib repo and one system template repo per area template:

- `system/stdlib`
- `system/template_default`
- `system/template_lpc`
- `system/template_rust`
- `system/template_kotlin_ktor`
- `system/template_kotlin_quarkus`
- `system/template_kotlin_spring_boot`

Responsibilities:

- `system/stdlib`
  - contains the active stdlib implementation
  - is loaded and hot-reloaded like a normal area
  - is editable through the existing git server and editor
  - includes stdlib web definitions, portal behavior, and any route/app declarations that currently live outside git
- `system/template_*`
  - each repo contains one template as a complete git repository
  - becomes the runtime source for template listing and new repo creation
  - can be edited, versioned, branched, and reviewed independently

## Bootstrap Rules

### First startup

If the system repos do not exist:

1. Load bootstrap templates from a bootstrap source, initially derived from the existing adapter-owned assets
2. Create one `system/template_*` repo for each built-in template
3. Create `system/stdlib` from the configured stdlib template
4. Check out all system repos into the world workspace

### Later startups

If the system repos already exist:

- do not use `adapters/*` as runtime template sources
- load templates only from `system/template_*` repos
- load stdlib only from `world/system/stdlib`

## Repository Layout Refactor

The repository layout should reflect the separation between runtime and content.

Desired direction:

- `adapters/`
  - Ruby runtime and MOP client
  - JVM launcher/runtime and MOP client
  - LPC/Rust runtime and MOP client
- `bootstrap/` or similar
  - optional built-in bootstrap stdlib/template snapshots used only to create missing system repos
- `data/world/system/*`
  - actual editable stdlib and template repos at runtime

This means the current adapter-owned stdlib and template files should eventually be moved out of `adapters/` entirely.

## Configuration

Add explicit bootstrap configuration for first startup:

- selected stdlib template, for example `MUD_STDLIB_TEMPLATE`
- selected set of area templates to provision on first startup
- optional bootstrap/template mode if needed later

Rules:

- bootstrap config only matters if the system repos are missing
- once the system repos exist, runtime should ignore adapter disk templates

Suggested config shape:

- `stdlib_template`
  - logical template name used to create `system/stdlib` on first boot
- `bootstrap_area_templates`
  - list of logical template names to provision as `system/template_*` repos on first boot

Suggested top-level section:

```yaml
bootstrap:
  stdlib_template: ruby
  area_templates:
    - default
    - lpc
    - kotlin:ktor
```

Behavior:

- if `bootstrap_area_templates` is omitted, provision a sensible built-in default set
- if it is present, only provision the named templates
- adding a new template later should normally be done by creating a new `system/template_*` repo, not by editing config
- config is for bootstrap selection, not long-term template registration
- if bootstrap of `system/stdlib` fails, startup should fail hard
- if bootstrap of one or more template repos fails, startup may continue, but it must log loudly and template-based repo creation may be unavailable for those templates

## Template Source Refactor

Refactor area template loading so it is not tied to adapter handshakes or disk scans under `adapters/*`.

Introduce two conceptual sources:

- `BootstrapTemplateSource`
  - reads bootstrap files from a dedicated bootstrap source
  - only used when provisioning missing system repos
- `SystemRepoTemplateSource`
  - reads templates from checked-out `system/template_*` workspaces
  - used during normal runtime

Keep the current in-memory representation initially:

- `HashMap<String, HashMap<String, String>>`

That minimizes churn while changing the source of truth.

## Structure For Template Repos

Each area template should be its own git repo under `system`.

Suggested mapping:

- template name `default` -> repo `system/template_default`
- template name `lpc` -> repo `system/template_lpc`
- template name `rust` -> repo `system/template_rust`
- template name `kotlin:ktor` -> repo `system/template_kotlin_ktor`
- template name `kotlin:quarkus` -> repo `system/template_kotlin_quarkus`
- template name `kotlin:spring-boot` -> repo `system/template_kotlin_spring_boot`

Naming convention:

- all template repos live under namespace `system`
- all template repos use the prefix `template_`
- repo name format:
  - `template_<logical_name_normalized>`
- normalization rules:
  - lowercase only
  - replace `:` with `_`
  - replace `-` with `_`
  - keep names ASCII

Examples:

- `default` -> `system/template_default`
- `lpc` -> `system/template_lpc`
- `rust` -> `system/template_rust`
- `kotlin:ktor` -> `system/template_kotlin_ktor`
- `kotlin:spring-boot` -> `system/template_kotlin_spring_boot`

Driver behavior:

- each template repo is treated as a complete repository snapshot
- `/api/repos/templates` should list templates by logical template name, not raw repo name
- creating a new area from a template should clone or fork that template repo into the builder's new repo
- creating a new template should normally just mean adding a new `system/template_*` repo that follows the naming convention
- template repos should carry a metadata file such as `template.yml`
- template metadata should be used for logical naming, UI display, language or framework description, and compatibility information rather than depending on repo name alone

Template discovery rule:

- runtime should discover available templates by listing `system/template_*` repos
- logical template names should be derived from repo names by reversing the normalization rules where possible, or by storing explicit metadata in the template repo
- config should not be required to make an already-existing `system/template_*` repo visible as a usable template

Suggested `template.yml` shape:

```yaml
name: kotlin:ktor
display_name: Kotlin Ktor
kind: area_template
language: kotlin
framework: ktor
branch: main
stdlib_compatible: true
description: Ktor-based area template
```

Required fields:

- `name`
- `kind`
- `language`

Optional fields:

- `framework`
- `display_name`
- `description`
- `branch`
- `stdlib_compatible`

## Stdlib As A Normal Area

`system/stdlib` should behave like any other area:

- discovered by area discovery
- checked out into workspace
- loaded by the normal load/reload path
- hot-reloaded on commit/push

This removes the special case where stdlib exists only inside adapter files.

## Stdlib Web Definitions

The stdlib migration must include web-facing stdlib code, not just world logic.

This includes:

- stdlib web route definitions
- stdlib web app declarations
- portal-related stdlib behavior that is currently sourced from adapter-owned files
- any template-rendering or web helper code that must be editable at runtime

Target behavior:

- the live source for stdlib web definitions is `system/stdlib`
- editing stdlib web code through git or the built-in editor updates runtime behavior
- reload behavior for stdlib web code follows the same git-driven workflow as area code
- runtime should not depend on adapter-owned copies of stdlib web definitions after bootstrap
- the full global portal is owned by stdlib, not by adapter-owned static app classes

Open design point:

- if some portal shell/runtime code must remain in `adapters/` for bootstrapping or process startup, the boundary must be explicit
- only runtime framework code should stay in `adapters/`
- editable stdlib web behavior should move into git
- the adapter should retain only Puma or Rack or MOP shell concerns and dynamically load the portal implementation from `system/stdlib`

## Repo Provisioning Changes

Harden seeded repo creation so templates always produce valid areas.

Guarantees:

- always ensure `.meta.yml` exists
- do not assume non-Ruby templates already contain discovery-critical files
- apply the same rules to both system repos and normal builder repos

## Runtime Behavior Changes

Change repo creation to use template repos under `system`.

Implications:

- `/api/repos/create` should create a new repo by cloning or forking the selected `system/template_*` repo
- internal `repo_create` should do the same
- template creation should stop being a file-copy operation and become a repo-copy operation
- runtime should not depend on adapter template files once bootstrap is complete
- adapter startup should not be responsible for publishing live templates during normal operation
- stdlib web and portal definitions should be resolved from `system/stdlib`, not from adapter-owned source files
- runtime reload logic should be able to classify stdlib diffs and only reload affected subsystems where possible

## Template Repo Cloning Model

New area creation should work like this:

1. user selects a logical template name
2. driver resolves that template name to a `system/template_*` repo
3. driver reads the template repo contents from the template repo's `main`
4. driver creates the destination bare repo for the builder
5. driver seeds the new repo with template content as a fresh initial commit
6. driver creates both `main` and `develop` in the new repo, initially pointing at the same seeded commit
7. driver checks out the new repo into the builder workspace

This should replace the current `HashMap<String, String>` seeding model over time.

Design target:

- template repos are first-class git repos
- new areas inherit the exact content state of the template
- template evolution happens by editing the template repo itself
- builder repos do not inherit template git history

## Access Control

Editing policy for system repos should be explicit.

Current direction:

- bootstrap write access to system repos defaults to admin only
- system repos are only visible to users who actually have access
- long-term ACL policy should be definable by stdlib rather than permanently hardcoded in the driver
- stdlib should own ACL policy for both system repos and normal builder area repos
- first implementation should have the driver ask stdlib at request time
- portal code should not be the place where ACL policy itself lives

## Implementation Phases

### Phase 1

- add bootstrap config for stdlib selection
- add bootstrap config for which templates are provisioned on first startup
- provision `system/stdlib`
- provision one `system/template_*` repo per built-in template
- make sure both repos are checked out on first startup

### Phase 2

- load area templates from `system/template_*` repos
- switch repo creation to clone or fork from template repos
- keep bootstrap assets only as a fallback when system repos are missing
- start resolving stdlib web definitions from `system/stdlib`

### Phase 3

- stop using `adapters/*` as runtime sources after bootstrap
- verify restart behavior
- tighten ACL behavior for system repos
- remove adapter-owned stdlib web definitions from the runtime path
- make driver-triggered stdlib reload propagate to dependent areas and objects

### Phase 4

- move bootstrap stdlib/template assets out of `adapters/`
- leave `adapters/` with runtime-only responsibilities

### Phase 5

Optional future improvement:

- preserve or rewrite template history depending on product needs

## Recommended Implementation Order

1. Add config/env for bootstrap stdlib selection
2. Add config/env for bootstrap template selection
3. Add helper functions for provisioning system repos
4. Add loader for templates from `system/template_*` repos
5. Switch repo creation to use template repo clone or fork
6. Make `system/stdlib` load and reload as a normal area
7. Move stdlib web definitions to be sourced from `system/stdlib`
8. Disable adapter template usage after bootstrap
9. Move bootstrap assets out of `adapters/`
10. Add ACL handling for system repos
11. Add restart and hot-reload tests

## Technical Breakdown

### Config And Bootstrap

Files likely involved:

- `crates/mud-driver/src/config.rs`
- `crates/mud-driver/src/main.rs`
- `crates/mud-driver/src/server.rs`

Concrete work:

- add a new top-level bootstrap config section
- support env overrides for bootstrap stdlib and bootstrap template list
- define first-boot provisioning rules for `system/stdlib` and `system/template_*`
- make bootstrap failure semantics explicit:
  - stdlib failure stops startup
  - template failure logs loudly and startup may continue

### Repo Provisioning

Files likely involved:

- `crates/mud-driver/src/git/repo_manager.rs`
- `crates/mud-driver/src/git/workspace.rs`
- `crates/mud-driver/src/server.rs`
- `crates/mud-driver/src/web/repos.rs`

Concrete work:

- add helpers for creating system repos on first startup
- add support for creating a new repo from a template repo snapshot rather than only from in-memory file maps
- ensure new repos always contain valid bootstrap metadata such as `.meta.yml`
- ensure new repos get both `main` and `develop`
- use template repo `main` as the seed source for new builder repos

### Template Registry

Files likely involved:

- `crates/mud-driver/src/server.rs`
- `crates/mud-driver/src/web/repos.rs`
- new loader module if needed under `crates/mud-driver/src/web/` or `crates/mud-driver/src/server/`

Concrete work:

- replace adapter-handshake and adapter-disk templates as the long-term source of truth
- load templates from checked-out `system/template_*` repos
- parse `template.yml`
- expose logical template info through `/api/repos/templates`
- keep temporary bootstrap fallback during migration

### Ruby Runtime Boundary

Files likely involved:

- `adapters/ruby/bin/mud-adapter`
- `adapters/ruby/lib/mud_adapter/client.rb`
- `adapters/ruby/lib/mud_adapter/web_server.rb`
- `adapters/ruby/lib/mud_adapter/area_loader.rb`
- new runtime loader modules under `adapters/ruby/lib/mud_adapter/`

Concrete work:

- stop hardcoding portal app classes from adapter-owned stdlib files
- introduce a dynamic stdlib runtime namespace loaded from `system/stdlib`
- keep adapter responsibility limited to runtime shell, Puma or Rack setup, and MOP plumbing
- add subsystem-based reload for:
  - world and commands
  - portal and web
  - shared base or helpers

### Portal Runtime Loading

Files likely involved:

- current portal files under `bootstrap/ruby/stdlib/portal/`
- future equivalents under `system/stdlib`
- `adapters/ruby/lib/mud_adapter/web_server.rb`

Concrete work:

- define how portal app entrypoints are discovered inside `system/stdlib`
- load routes, views, helpers, and app modules dynamically
- reload portal subsystem in-process when portal-related stdlib files change
- keep unrelated world edits from forcing portal reload

### Driver Reload Propagation

Files likely involved:

- `crates/mud-driver/src/server.rs`
- `crates/mud-driver/src/runtime/state_store.rs`
- `crates/mud-driver/src/runtime/version_tree.rs`
- `crates/mud-driver/src/runtime/object_broker.rs`
- `crates/mud-driver/src/mop_rpc.rs`

Concrete work:

- classify stdlib changes by subsystem
- have the driver initiate stdlib reload automatically on commit or push
- first version: propagate reload at subsystem granularity
- later version: leave room for more explicit stdlib dependency tracking

### ACL Ownership

Files likely involved:

- `crates/mud-driver/src/git/repo_manager.rs`
- `crates/mud-driver/src/web/git_http.rs`
- `crates/mud-driver/src/web/editor_files.rs`
- `crates/mud-driver/src/server.rs`
- stdlib-side policy code under `system/stdlib`

Concrete work:

- replace driver-owned final ACL decisions with stdlib-owned policy decisions
- first implementation: driver asks stdlib at access-check time
- apply this to both system repos and builder area repos
- keep a safe bootstrap default before stdlib policy is available

### Migration Cleanup

Files likely involved:

- `adapters/ruby/...`
- `adapters/jvm/...`
- `adapters/lpc/...`
- any bootstrap assets moved to a future `bootstrap/` tree

Concrete work:

- keep adapter-owned templates only as temporary migration fallback
- remove runtime dependence on adapter-owned stdlib and template content
- move bootstrap content out of `adapters/` after the new flow is stable

## Sprint 1

Sprint 1 should focus on foundation work in the driver and git layers, without yet attempting the full Ruby portal runtime loader refactor.

Goal:

- make bootstrap and system repo provisioning real
- make template repos a first-class concept in the driver
- keep existing adapter-driven runtime behavior working while the new system repos are introduced

### Sprint 1 Scope

In scope:

- new bootstrap config
- first-boot provisioning of `system/stdlib`
- first-boot provisioning of `system/template_*` repos
- template metadata parsing from `template.yml`
- template discovery from system repos
- repo creation from template repo `main`
- fresh initial commit in new builder repos
- creation of both `main` and `develop` in new builder repos
- tests for provisioning and template discovery

Out of scope:

- full Ruby stdlib dynamic portal loader
- stdlib-owned ACL via RPC
- subsystem-based stdlib reload propagation
- removal of all adapter-owned bootstrap assets

### Sprint 1 Work Order

1. Add bootstrap config model

Files:

- `crates/mud-driver/src/config.rs`

Work:

- add top-level `bootstrap` config section
- add fields for:
  - `stdlib_template`
  - `area_templates`
- add tests for YAML parsing
- add env override handling if that already exists centrally, otherwise document it for the next step

Deliverable:

- config can express first-boot stdlib and template provisioning choices

2. Introduce system repo naming helpers

Files:

- `crates/mud-driver/src/server.rs`
- optionally a new helper module under `crates/mud-driver/src/git/` or `crates/mud-driver/src/web/`

Work:

- add helper for mapping logical template names to `system/template_*` repo names
- centralize normalization rules
- avoid scattering string conventions across server and web code

Deliverable:

- one authoritative naming path for template repo resolution

3. Extend repo manager for template-based repo creation

Files:

- `crates/mud-driver/src/git/repo_manager.rs`

Work:

- add a new path for seeding a repo from another repo snapshot rather than only from file maps
- read source content from template repo `main`
- create a fresh destination repo with:
  - new initial commit
  - `main`
  - `develop`
- keep existing in-memory template seeding temporarily for migration fallback

Deliverable:

- driver can create a new bare repo from a template repo snapshot

4. Add first-boot system repo provisioning

Files:

- `crates/mud-driver/src/server.rs`
- possibly `crates/mud-driver/src/git/workspace.rs`

Work:

- after repo manager and workspace are initialized, ensure:
  - `system/stdlib`
  - selected `system/template_*` repos
- provision missing repos only
- check out provisioned repos into workspace
- fail hard if stdlib bootstrap fails
- log loudly and continue if template repo bootstrap fails

Deliverable:

- first startup creates system repos on disk in a deterministic way

5. Add template metadata loading

Files:

- `crates/mud-driver/src/server.rs`
- `crates/mud-driver/src/web/repos.rs`
- optionally a new template loader module

Work:

- define a Rust struct for `template.yml`
- load metadata from checked-out `system/template_*` workspaces
- expose logical template list through existing templates API
- keep adapter template fallback for repos that do not yet exist during migration

Deliverable:

- `/api/repos/templates` can be backed by system template repos

6. Switch repo creation API to system template repos

Files:

- `crates/mud-driver/src/web/repos.rs`
- `crates/mud-driver/src/server.rs`

Work:

- resolve selected template by logical name
- map it to the corresponding `system/template_*` repo
- create builder repos from template repo `main`
- preserve existing API surface where practical

Deliverable:

- creating a repo from a template no longer depends on adapter-owned file maps as the long-term source

7. Add tests for the new provisioning flow

Files:

- `crates/mud-driver/tests/...`
- `crates/mud-driver/src/git/repo_manager.rs` tests
- `crates/mud-driver/src/config.rs` tests

Work:

- test bootstrap config parsing
- test template repo naming normalization
- test creating a repo from a template repo produces:
  - fresh history
  - both `main` and `develop`
  - expected seeded files
- test first-boot provisioning of `system/stdlib`
- test first-boot provisioning of configured `system/template_*`
- test template bootstrap failure is non-fatal
- test stdlib bootstrap failure is fatal

Deliverable:

- the system repo provisioning model is covered before runtime loader work begins

### Sprint 1 Exit Criteria

Sprint 1 is done when all of the following are true:

- bootstrap config exists and is tested
- first boot provisions `system/stdlib`
- first boot provisions configured `system/template_*` repos
- template repos are discoverable through metadata
- new builder repos can be created from template repo `main`
- new builder repos start with fresh history and both `main` and `develop`
- existing runtime can still boot without the full portal-loader refactor being done

### Sprint 1 Risks

- current code mixes template registration with adapter startup, so dual-running fallback logic may become messy if not isolated early
- repo seeding from template repos must be implemented carefully so fresh-history behavior is correct
- template discovery should be introduced behind a small abstraction, otherwise `server.rs` will absorb too much new logic

### Sprint 1 Suggested Order Of Execution

1. `config.rs`
2. template naming helper
3. `repo_manager.rs`
4. provisioning logic in `server.rs`
5. template metadata loader
6. `web/repos.rs` and internal repo creation paths
7. targeted tests

## Test Coverage Needed

- first boot creates `system/stdlib`
- first boot creates all expected `system/template_*` repos
- configured stdlib template is used only during bootstrap
- configured bootstrap template list controls which template repos are created on first boot
- restart does not use adapter disk templates if system repos already exist
- restart does not depend on `adapters/*` content repos existing
- `system/stdlib` is discoverable as an area
- commit or push to `system/stdlib` triggers reload
- commit or push to stdlib web definitions updates runtime web behavior
- stdlib reload can be scoped based on diff classification
- stdlib reload can propagate to dependent loaded areas or objects
- `/api/repos/templates` reflects templates from `system/template_*` repos
- creating a new repo from a template still works for Ruby, LPC, and JVM templates
- creating a new repo from a template copies the template repo contents correctly
- adding a new `system/template_*` repo makes a new template available without config changes

## Notes

- The current code already has most of the primitives needed: repo creation, workspace checkout, git HTTP, hot reload paths, and template registration.
- The main refactor is to move the source of truth from adapter-owned files to system-owned repos.
- A second important refactor is architectural: `adapters/` should become runtime-only, not content-bearing.
- This should be done incrementally so repo creation and area loading remain working throughout the transition.
