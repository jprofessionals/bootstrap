# MUD Driver (Rust) — development commands

# Configuration
db_user := "mud_admin"
db_password := "muddev"
db_host := "localhost"
db_port := "5432"
db_container := "mud-postgres"
config_file := "config/server.yml"
ruby_adapter_dir := "adapters/ruby"
jvm_adapter_dir := "adapters/jvm"
bundle := env("BUNDLE", `which bundle 2>/dev/null || echo "$HOME/.local/share/gem/ruby/3.4.0/bin/bundle"`)

# Default: list available recipes
default:
    @just --list

# ─── Environment ─────────────────────────────────────────────────────

# Start the full development environment (postgres + build + run)
up: db-start setup build
    just dev

# Stop the full development environment
down: kill db-stop

# Kill orphaned mud-driver and mud-adapter processes
kill:
    #!/usr/bin/env bash
    set +e
    pids=$(pgrep -f 'target/.*mud.driver' 2>/dev/null)
    if [ -n "$pids" ]; then
        echo "$pids" | xargs kill 2>/dev/null
        echo "Stopped mud-driver"
    else
        echo "No mud-driver running"
    fi
    pids=$(pgrep -f 'mud-adapter.*--socket' 2>/dev/null)
    if [ -n "$pids" ]; then
        echo "$pids" | xargs kill 2>/dev/null
        echo "Stopped mud-adapter"
    else
        echo "No mud-adapter running"
    fi
    rm -f /tmp/mud-driver-*.sock 2>/dev/null
    exit 0

# Set up directories, config, and Ruby adapter gems
setup: setup-dirs setup-config setup-ruby

# Create world and git-server directories if missing
setup-dirs:
    @mkdir -p data/world data/git-server config

# Generate dev config if it doesn't exist
setup-config:
    @if [ ! -f {{config_file}} ]; then \
        echo 'server_name: "Rust MUD (dev)"' > {{config_file}}; \
        echo 'ssh:' >> {{config_file}}; \
        echo '  host: "127.0.0.1"' >> {{config_file}}; \
        echo '  port: 2222' >> {{config_file}}; \
        echo 'http:' >> {{config_file}}; \
        echo '  host: "127.0.0.1"' >> {{config_file}}; \
        echo '  port: 8080' >> {{config_file}}; \
        echo '  enabled: true' >> {{config_file}}; \
        echo 'world:' >> {{config_file}}; \
        echo '  path: "world"' >> {{config_file}}; \
        echo '  git_path: "git-server"' >> {{config_file}}; \
        echo 'database:' >> {{config_file}}; \
        echo '  host: "{{db_host}}"' >> {{config_file}}; \
        echo '  port: {{db_port}}' >> {{config_file}}; \
        echo '  admin_user: "{{db_user}}"' >> {{config_file}}; \
        echo '  admin_password: "{{db_password}}"' >> {{config_file}}; \
        echo 'adapters:' >> {{config_file}}; \
        echo '  ruby:' >> {{config_file}}; \
        echo '    enabled: true' >> {{config_file}}; \
        echo '    command: "ruby"' >> {{config_file}}; \
        echo '    adapter_path: "adapters/ruby/bin/mud-adapter"' >> {{config_file}}; \
        echo "Created {{config_file}}"; \
    else \
        echo "{{config_file}} already exists"; \
    fi

# Install Ruby adapter gems (bundle install)
setup-ruby:
    cd {{ruby_adapter_dir}} && {{bundle}} install --quiet

# ─── Database (PostgreSQL via Docker) ────────────────────────────────

# Start PostgreSQL in Docker
db-start:
    @if docker ps --format '{{"{{.Names}}"}}' | grep -q '^{{db_container}}$'; then \
        echo "PostgreSQL already running"; \
    elif docker ps -a --format '{{"{{.Names}}"}}' | grep -q '^{{db_container}}$'; then \
        echo "Starting existing PostgreSQL container..."; \
        docker start {{db_container}}; \
    else \
        echo "Creating PostgreSQL container..."; \
        docker run -d \
            --name {{db_container}} \
            -e POSTGRES_USER={{db_user}} \
            -e POSTGRES_PASSWORD={{db_password}} \
            -e POSTGRES_DB=postgres \
            -p {{db_port}}:5432 \
            postgres:16-alpine; \
    fi
    @echo "Waiting for PostgreSQL to be ready..."
    @for i in $(seq 1 30); do \
        docker exec {{db_container}} pg_isready -U {{db_user}} > /dev/null 2>&1 && break; \
        sleep 1; \
    done
    @docker exec {{db_container}} pg_isready -U {{db_user}} > /dev/null 2>&1 && \
        echo "PostgreSQL ready on port {{db_port}}" || \
        (echo "PostgreSQL failed to start" && exit 1)

# Stop PostgreSQL container
db-stop:
    @docker stop {{db_container}} 2>/dev/null && echo "PostgreSQL stopped" || echo "PostgreSQL not running"

# Remove PostgreSQL container and its data
db-destroy:
    @docker rm -f {{db_container}} 2>/dev/null && echo "PostgreSQL container removed" || echo "No container to remove"

# Open a psql shell to the driver database
db-shell db="mud_driver":
    docker exec -it {{db_container}} psql -U {{db_user}} -d {{db}}

# Show database status
db-status:
    @docker ps --filter name={{db_container}} --format "table {{"{{.Names}}"}}\t{{"{{.Status}}"}}\t{{"{{.Ports}}"}}" 2>/dev/null || echo "Container not found"

# ─── Build & Run ─────────────────────────────────────────────────────

# Build the entire workspace
build:
    cargo build --workspace

# Build in release mode
release:
    cargo build --workspace --release

# Run the MUD driver with dev config
dev:
    RUST_LOG=info cargo run -p mud-driver -- --config {{config_file}}

# Run the MUD driver (custom args)
run *args:
    cargo run -p mud-driver -- {{args}}

# Run check (fast compile check without codegen)
check:
    cargo check --workspace

# Clean everything for a fresh start (build artifacts, databases, world data)
clean:
    #!/usr/bin/env bash
    set +e
    # Stop running processes
    pids=$(pgrep -f 'target/.*mud.driver' 2>/dev/null)
    [ -n "$pids" ] && echo "$pids" | xargs kill 2>/dev/null && echo "Stopped mud-driver"
    pids=$(pgrep -f 'mud-adapter.*--socket' 2>/dev/null)
    [ -n "$pids" ] && echo "$pids" | xargs kill 2>/dev/null && echo "Stopped mud-adapter"
    rm -f /tmp/mud-driver-*.sock 2>/dev/null
    # Clean build artifacts
    cargo clean
    # Destroy database container
    docker rm -f {{db_container}} 2>/dev/null && echo "PostgreSQL container removed" || true
    # Remove world data and config
    rm -rf data world git-server && echo "Removed data/, world/, and git-server/"
    rm -f {{config_file}} && echo "Removed {{config_file}}"
    echo "Clean complete — run 'just up' for a fresh start"

# ─── Testing ─────────────────────────────────────────────────────────

# Run all tests (Rust + Ruby + E2E)
test: test-rust test-ruby test-e2e

# Run Rust tests (excludes E2E which needs Docker image build)
test-rust:
    cargo test --workspace --exclude mud-e2e

# Run tests for a specific crate
test-crate crate:
    cargo test -p {{crate}}

# Run a specific test by name
test-one name:
    cargo test --workspace '{{name}}'

# Run tests with output shown
test-verbose:
    cargo test --workspace -- --nocapture

# Run E2E tests (requires Docker — builds image via harness, cached by Docker layers)
test-e2e:
    cargo test -p mud-e2e -- --nocapture

# Run tests that need PostgreSQL (starts DB first)
test-db: db-start
    MUD_TEST_POSTGRES_URL="postgres://{{db_user}}:{{db_password}}@{{db_host}}:{{db_port}}/postgres" cargo test --workspace

# Run Ruby adapter tests
test-ruby:
    cd {{ruby_adapter_dir}} && {{bundle}} exec ruby test/portal_base_app_test.rb

# Run JVM adapter tests (Gradle)
test-jvm:
    cd {{jvm_adapter_dir}} && ./gradlew test

# Build the JVM adapter (compile only)
build-jvm:
    cd {{jvm_adapter_dir}} && ./gradlew build

# Build the JVM launcher fat JAR
build-jvm-jar:
    cd {{jvm_adapter_dir}} && ./gradlew :launcher:jar

# Clean JVM adapter build artifacts
clean-jvm:
    cd {{jvm_adapter_dir}} && ./gradlew clean

# Show test count summary
test-count:
    @cargo test --workspace 2>&1 | grep -E "^(running|test result)" | grep "test result" | awk '{sum += $4} END {print "Total: " sum " tests"}'

# Watch for changes and run tests (requires cargo-watch)
watch:
    cargo watch -x 'test --workspace'

# ─── Linting & Formatting ───────────────────────────────────────────

# Run clippy on the entire workspace
clippy:
    cargo clippy --workspace

# Run clippy and fail on warnings (for CI)
clippy-strict:
    cargo clippy --workspace -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Full CI check: format, clippy, tests
ci: fmt-check clippy-strict test

# ─── Documentation ───────────────────────────────────────────────────

# Generate documentation
doc:
    cargo doc --workspace --no-deps

# Open documentation in browser
doc-open:
    cargo doc --workspace --no-deps --open

# ─── Status ──────────────────────────────────────────────────────────

# Show the status of all services
status:
    @echo "── PostgreSQL ──"
    @docker ps --filter name={{db_container}} --format "  {{"{{.Status}}"}}" 2>/dev/null || echo "  Not running"
    @echo "── Ports ──"
    @echo "  SSH:  2222"
    @echo "  HTTP: 8080"
    @echo "  Ruby Portal: 8081"
    @echo "  PostgreSQL:  {{db_port}}"
    @echo "── Config ──"
    @echo "  {{config_file}}"
