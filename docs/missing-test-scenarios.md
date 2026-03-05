# Missing Test Scenarios — Rust Driver vs Ruby Driver

Comparison of Ruby MUD driver test suite (~502 tests) against the Rust MUD driver test suite (~166 tests).
Focuses on driver-level functionality; excludes Ruby stdlib (world objects, commands, player DSL)
which would live in the Ruby adapter, not the Rust driver.

**What Rust already covers well:** MOP protocol, adapter management, config validation,
git repo/workspace/branch-protection, HTTP git protocol, session lifecycle, credential encryption,
review policy unit tests, area discovery, and full-stack E2E flow.

---

## 1. Configuration — Network Settings

| # | Scenario | Ruby Source |
|---|----------|------------|
| 1 | Network defaults (enabled, outbound, inbound, logging) | `test_config_network.rb` |
| 2 | Network config from hash | `test_config_network.rb` |
| 3 | HTTP threads default (1) | `test_config_network.rb` |
| 4 | HTTP threads from hash | `test_config_network.rb` |
| 5 | Outbound defaults (host allow/blocklists) | `test_config_network.rb` |
| 6 | Inbound defaults (port allowlist) | `test_config_network.rb` |
| 7 | Logging defaults | `test_config_network.rb` |

## 2. Router / Game State

| # | Scenario | Ruby Source |
|---|----------|------------|
| 8 | Register area with router | `test_router.rb` |
| 9 | Place player in room | `test_router.rb` |
| 10 | Query players in a room | `test_router.rb` |
| 11 | Move player between rooms | `test_router.rb` |
| 12 | Blocked movement on invalid direction | `test_router.rb` |
| 13 | Remove player from game | `test_router.rb` |
| 14 | Track online player count | `test_router.rb` |
| 15 | Find area by namespace/name | `test_router.rb` |

*Note: Rust has session management but no in-driver game router. May be intentional if routing lives in the adapter.*

## 3. Tick Engine Runtime

| # | Scenario | Ruby Source |
|---|----------|------------|
| 16 | Tick publishes event | `test_tick_engine.rb` |
| 17 | Register and run daemon | `test_tick_engine.rb` |
| 18 | Daemon respects its interval | `test_tick_engine.rb` |
| 19 | Start and stop engine | `test_tick_engine.rb` |

*Rust validates tick config but has no tick engine runtime tests.*

## 4. Hot Reload / File Watching

| # | Scenario | Ruby Source |
|---|----------|------------|
| 20 | Detect newly created file | `test_hot_reload.rb` |
| 21 | Detect modified file | `test_hot_reload.rb` |
| 22 | Detect deleted file | `test_hot_reload.rb` |
| 23 | Only watch language-specific files | `test_hot_reload.rb` |

## 5. Reload Manager

| # | Scenario | Ruby Source |
|---|----------|------------|
| 24 | Reload triggers area reload callback | `test_reload_manager.rb` |
| 25 | Reload syncs workspace | `test_reload_manager.rb` |
| 26 | Reload writes success log | `test_reload_manager.rb` |
| 27 | Reload writes error log on failure | `test_reload_manager.rb` |
| 28 | Reload skips unknown area | `test_reload_manager.rb` |
| 29 | Reload develop updates dev checkout | `test_reload_manager.rb` |
| 30 | Reload main updates production checkout | `test_reload_manager.rb` |
| 31 | Default reload targets develop branch | `test_reload_manager.rb` |

*Rust has area reload via MOP in full_stack_e2e but no dedicated reload manager tests.*

## 6. Area Template

| # | Scenario | Ruby Source |
|---|----------|------------|
| 32 | Render creates shared files (.meta.yml, mud_aliases, etc.) | `test_area_template.rb` |
| 33 | Render creates ERB web files | `test_area_template.rb` |
| 34 | Render creates SPA files + dev toolkit | `test_area_template.rb` |
| 35 | Makes bin scripts executable | `test_area_template.rb` |
| 36 | Placeholder substitution (namespace, area_name) | `test_area_template.rb` |
| 37 | .meta.yml placeholder substitution | `test_area_template.rb` |
| 38 | Default mode is ERB with SPA commented | `test_area_template.rb` |
| 39 | Static files not corrupted after render | `test_area_template.rb` |
| 40 | Render from stdlib checkout path | `test_area_template.rb` |

## 7. Player Store — Personal Access Tokens

| # | Scenario | Ruby Source |
|---|----------|------------|
| 41 | Create token and authenticate with it | `test_player_store_tokens.rb` |
| 42 | Wrong token returns failure | `test_player_store_tokens.rb` |
| 43 | Revoke token then auth fails | `test_player_store_tokens.rb` |
| 44 | List tokens shows metadata not hashes | `test_player_store_tokens.rb` |
| 45 | Token prefix is first 8 chars | `test_player_store_tokens.rb` |
| 46 | last_used_at updates on auth | `test_player_store_tokens.rb` |
| 47 | Cannot create token for nonexistent player | `test_player_store_tokens.rb` |
| 48 | Revoke nonexistent token returns false | `test_player_store_tokens.rb` |
| 49 | Token auth for nonexistent player fails | `test_player_store_tokens.rb` |

## 8. Player Store — Characters (error cases)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 50 | Find returns account role | `test_player_store_characters.rb` |
| 51 | Add character to unknown player fails | `test_player_store_characters.rb` |
| 52 | Switch to unknown character name fails | `test_player_store_characters.rb` |
| 53 | Switch character for unknown player fails | `test_player_store_characters.rb` |

*Rust account_auth_test covers happy-path add/switch but not these error cases.*

## 9. Player Store — SSH Keys & Roles

| # | Scenario | Ruby Source |
|---|----------|------------|
| 54 | Add SSH key to player | `test_player_store.rb` |
| 55 | Default account role is 'player' | `test_player_store_characters.rb` |
| 56 | Account role for unknown player returns nil | `test_player_store_characters.rb` |
| 57 | Set role (e.g., 'builder') | `test_player_store_characters.rb` |
| 58 | Set role for unknown player fails | `test_player_store_characters.rb` |
| 59 | Set builder character | `test_player_store_characters.rb` |
| 60 | Set builder character rejects non-builder account | `test_player_store_characters.rb` |

## 10. Player Store — Basic (gaps)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 61 | Find nonexistent player returns nil | `test_player_store.rb` |
| 62 | List all players | `test_player_store.rb` |

*Rust tests password hashing in unit tests but not DB-level find/list operations.*

## 11. Area Registry (DB-backed)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 63 | Register area in database | `test_area_registry.rb` |
| 64 | Find area metadata by namespace/name | `test_area_registry.rb` |
| 65 | Find returns nil for unknown area | `test_area_registry.rb` |
| 66 | Update area metadata (owner) | `test_area_registry.rb` |
| 67 | List all registered areas | `test_area_registry.rb` |
| 68 | Unregister area | `test_area_registry.rb` |

## 12. Merge Request Store (DB CRUD)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 69 | Create MR with default state 'open' | `test_merge_request_store.rb` |
| 70 | Find MR by ID | `test_merge_request_store.rb` |
| 71 | Find returns nil for missing ID | `test_merge_request_store.rb` |
| 72 | Add approval to MR | `test_merge_request_store.rb` |
| 73 | Add approval without comment | `test_merge_request_store.rb` |
| 74 | List MRs filtered by state | `test_merge_request_store.rb` |
| 75 | List without state filter returns all | `test_merge_request_store.rb` |
| 76 | List filters by namespace and area | `test_merge_request_store.rb` |
| 77 | Update MR state | `test_merge_request_store.rb` |
| 78 | Update state rejects invalid values | `test_merge_request_store.rb` |
| 79 | List approvals for a MR | `test_merge_request_store.rb` |
| 80 | List returns newest first | `test_merge_request_store.rb` |

*Rust has struct/clone/debug tests but no DB-backed CRUD tests.*

## 13. Merge Request Manager (workflow)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 81 | Create MR (develop → main) | `test_merge_request_manager.rb` |
| 82 | Create MR with description | `test_merge_request_manager.rb` |
| 83 | Create MR blocked by policy | `test_merge_request_manager.rb` |
| 84 | Execute merge with 0 required approvals | `test_merge_request_manager.rb` |
| 85 | Execute merge blocked by required approvals | `test_merge_request_manager.rb` |
| 86 | Full approve-and-merge workflow | `test_merge_request_manager.rb` |
| 87 | Approve rejected by policy | `test_merge_request_manager.rb` |
| 88 | Approve on non-open MR fails | `test_merge_request_manager.rb` |
| 89 | Approve nonexistent MR fails | `test_merge_request_manager.rb` |
| 90 | Reject MR | `test_merge_request_manager.rb` |
| 91 | Close MR | `test_merge_request_manager.rb` |
| 92 | Reopen rejected MR | `test_merge_request_manager.rb` |

*Rust has ReviewPolicy unit tests but no manager-level workflow tests.*

## 14. Database Manager (gaps)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 93 | Initialize stdlib database | `test_database_manager.rb` |
| 94 | Run stdlib migrations (players, characters) | `test_database_manager.rb` |
| 95 | Provision per-area database | `test_database_manager.rb` |
| 96 | Connect to area database | `test_database_manager.rb` |
| 97 | Provision area DB is idempotent | `test_database_manager.rb` |
| 98 | Run area migrations | `test_database_manager.rb` |
| 99 | Area migrations are idempotent | `test_database_manager.rb` |
| 100 | Drop area database | `test_database_manager.rb` |

*Rust tests migration SQL correctness but not DatabaseManager provisioning/connect/drop.*

## 15. Logging System

| # | Scenario | Ruby Source |
|---|----------|------------|
| 101 | Driver event goes to raw + driver log | `test_log_router.rb` |
| 102 | Driver event buffered in memory | `test_log_router.rb` |
| 103 | Driver event NOT in area buffer | `test_log_router.rb` |
| 104 | Area event goes to raw + area log | `test_log_router.rb` |
| 105 | Area event NOT in driver log | `test_log_router.rb` |
| 106 | Area event buffered in area buffer | `test_log_router.rb` |
| 107 | Register area creates log buffer | `test_log_router.rb` |
| 108 | Filter area entries by source | `test_log_router.rb` |
| 109 | Ring buffer overflow drops oldest | `test_log_buffer.rb` |
| 110 | Filter by source / severity / both | `test_log_buffer.rb` |
| 111 | Clear buffer | `test_log_buffer.rb` |
| 112 | Entries returns copy (not reference) | `test_log_buffer.rb` |
| 113 | Default capacity is 200 | `test_log_buffer.rb` |
| 114 | Writer formats entry with area prefix | `test_log_writer.rb` |
| 115 | Severity labels [INFO] [WARN] [ERROR] | `test_log_writer.rb` |
| 116 | Creates parent directory | `test_log_writer.rb` |
| 117 | Rotates log when size exceeded | `test_log_writer.rb` |
| 118 | Rotation chain (multiple files) | `test_log_writer.rb` |
| 119 | Max rotated files respected | `test_log_writer.rb` |

## 16. Network Manager / SSRF Protection

| # | Scenario | Ruby Source |
|---|----------|------------|
| 120 | Reject host in blocklist | `test_manager.rb` (network) |
| 121 | Reject host not in allowlist | `test_manager.rb` (network) |
| 122 | Block private IPs — loopback (127.0.0.1) | `test_manager.rb` (network) |
| 123 | Block private IPs — link-local (169.254.x.x) | `test_manager.rb` (network) |
| 124 | Block private IPs — RFC1918 (10.x, 192.168.x) | `test_manager.rb` (network) |
| 125 | Reject invalid URL without host | `test_manager.rb` (network) |
| 126 | All HTTP methods validate hosts | `test_manager.rb` (network) |
| 127 | Reject inbound port not in allowlist | `test_manager.rb` (network) |
| 128 | Reject driver port conflict (SSH) | `test_manager.rb` (network) |

## 17. Web App / Static File Server

| # | Scenario | Ruby Source |
|---|----------|------------|
| 129 | Serve static file with correct MIME type | `test_web_app.rb`, `test_static_file_server.rb` |
| 130 | Serve JavaScript files | `test_static_file_server.rb` |
| 131 | Area not found returns 404 | `test_web_app.rb` |
| 132 | Dev area (@dev) accessible | `test_web_app.rb` |
| 133 | Custom GET/POST routes | `test_web_app.rb` |
| 134 | Path traversal blocked (../) | `test_web_app.rb`, `test_static_file_server.rb` |
| 135 | SPA mode serves index.html at root | `test_web_app.rb` |
| 136 | ETag from mtime and size | `test_static_file_server.rb` |
| 137 | ERB mode cache headers | `test_static_file_server.rb` |
| 138 | Immutable cache for fingerprinted assets | `test_static_file_server.rb` |
| 139 | SPA root no-cache headers | `test_static_file_server.rb` |
| 140 | 304 Not Modified when ETag matches | `test_static_file_server.rb` |
| 141 | No 304 when ETag differs | `test_static_file_server.rb` |

## 18. SPA API App

| # | Scenario | Ruby Source |
|---|----------|------------|
| 142 | GET returns JSON | `test_spa_api_app.rb` |
| 143 | POST route works | `test_spa_api_app.rb` |
| 144 | PUT route works | `test_spa_api_app.rb` |
| 145 | DELETE route works | `test_spa_api_app.rb` |
| 146 | 404 returns JSON error | `test_spa_api_app.rb` |
| 147 | CORS headers on same-origin | `test_spa_api_app.rb` |
| 148 | CORS rejected for different origin | `test_spa_api_app.rb` |
| 149 | CORS preflight OPTIONS | `test_spa_api_app.rb` |

## 19. SSH Server (gaps)

| # | Scenario | Ruby Source |
|---|----------|------------|
| 150 | Password authenticator callback | `test_ssh_server.rb` |
| 151 | Is git command detection | `test_ssh_server.rb` |
| 152 | Parse git command string | `test_ssh_server.rb` |

## 20. JS Build Manager

| # | Scenario | Ruby Source |
|---|----------|------------|
| 153 | Compute build directory path | `test_js_build_manager.rb` |
| 154 | Compute dist directory path | `test_js_build_manager.rb` |
| 155 | Detect SPA source directory (true/false) | `test_js_build_manager.rb` |
| 156 | Build returns failure when no source | `test_js_build_manager.rb` |
| 157 | Copies package.json to build dir | `test_js_build_manager.rb` |
| 158 | Skips npm install when package.json unchanged | `test_js_build_manager.rb` |
| 159 | Returns failure on npm error | `test_js_build_manager.rb` |

## 21. Web Helpers

| # | Scenario | Ruby Source |
|---|----------|------------|
| 160 | Server name helper | `test_web_helpers.rb` |
| 161 | Total players online count | `test_web_helpers.rb` |

## 22. E2E — SSH Authentication

| # | Scenario | Ruby Source |
|---|----------|------------|
| 162 | SSH password auth succeeds | `test_end_to_end.rb` |
| 163 | SSH wrong password rejected | `test_end_to_end.rb` |
| 164 | SSH unknown user rejected | `test_end_to_end.rb` |
| 165 | SSH shell command dispatched | `test_end_to_end.rb` |
| 166 | Git clone over SSH works | `test_end_to_end.rb` |

## 23. E2E — Web Portal

| # | Scenario | Ruby Source |
|---|----------|------------|
| 167 | Welcome page loads | `test_portal_e2e.rb` |
| 168 | Login sets session cookie | `test_portal_e2e.rb` |
| 169 | Characters page shows characters | `test_portal_e2e.rb` |
| 170 | WebSocket terminal connection | `test_portal_e2e.rb` |
| 171 | Editor file CRUD API | `test_portal_e2e.rb` |
| 172 | Git workflow API (status, log, diff, commit) | `test_portal_e2e.rb` |

## 24. E2E — SPA

| # | Scenario | Ruby Source |
|---|----------|------------|
| 173 | SPA root serves index.html | `test_spa_e2e.rb` |
| 174 | SPA API GET | `test_spa_e2e.rb` |
| 175 | SPA API POST | `test_spa_e2e.rb` |
| 176 | SPA API 404 returns JSON | `test_spa_e2e.rb` |
| 177 | SPA CORS headers same-origin | `test_spa_e2e.rb` |

## 25. E2E — Branch Lifecycle / Merge Requests

| # | Scenario | Ruby Source |
|---|----------|------------|
| 178 | Create and merge MR with 0 approvals | `test_branch_lifecycle_e2e.rb` |
| 179 | Merge blocked when approvals insufficient | `test_branch_lifecycle_e2e.rb` |
| 180 | Full approve-and-merge workflow | `test_branch_lifecycle_e2e.rb` |

---

## Summary by Category

| Category | Missing Tests | Priority |
|----------|--------------|----------|
| Merge request store (DB CRUD) | 12 | High |
| Merge request manager (workflow) | 12 | High |
| Logging system | 19 | High |
| Player store — tokens (PAT) | 9 | High |
| Network/SSRF protection | 9 | High |
| Area template | 9 | Medium |
| Web app / static files | 13 | Medium |
| SPA API app | 8 | Medium |
| Reload manager | 8 | Medium |
| Database manager (gaps) | 8 | Medium |
| Router / game state | 8 | Medium |
| Player store — SSH keys & roles | 7 | Medium |
| Config — network | 7 | Medium |
| Area registry | 6 | Medium |
| JS build manager | 7 | Medium |
| E2E — web portal | 6 | Low |
| E2E — SSH auth | 5 | Low |
| E2E — SPA | 5 | Low |
| Tick engine runtime | 4 | Low |
| Hot reload | 4 | Low |
| Player store — characters (errors) | 4 | Low |
| E2E — branch lifecycle | 3 | Low |
| SSH server (gaps) | 3 | Low |
| Player store — basic (gaps) | 2 | Low |
| Web helpers | 2 | Low |
| **Total** | **~180** | |

*Priority based on: High = core driver functionality with no coverage, Medium = important but partially covered or less critical, Low = E2E/integration or features not yet implemented in Rust.*
