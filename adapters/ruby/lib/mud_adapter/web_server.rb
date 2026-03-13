# frozen_string_literal: true

require "puma"
require "puma/server"
require "rack"
require "rack/session"

module MudAdapter
  # Embedded HTTP server that serves the portal web apps (Account, Editor,
  # Git Dashboard, Review, Play). Runs Puma on a Unix socket in a background
  # thread alongside the main MOP message loop.
  #
  # The portal Roda apps call back to the Rust driver via the MOP client's
  # {Client#send_driver_request} for player auth, git operations, and MR
  # management.
  class WebServer
    attr_reader :socket_path

    def initialize(client:, socket_path: "/tmp/mud-portal.sock", session_secret: nil, session_handler: nil, area_loader: nil)
      @client = client
      @socket_path = socket_path
      @session_secret = session_secret || generate_session_secret
      @session_handler = session_handler
      @area_loader = area_loader
      @server = nil
      @thread = nil
    end

    # Start the web server in a background thread.
    # Returns immediately; the server runs until {#stop} is called.
    def start
      configure_portal_apps
      app = build_rack_app

      # Clean up stale socket file from a previous run.
      File.delete(@socket_path) if File.exist?(@socket_path)

      @server = Puma::Server.new(app)
      @server.add_unix_listener(@socket_path)

      @thread = Thread.new do
        @server.run.join
      rescue StandardError => e
        $stderr.puts("[web-server] Fatal error: #{e.class}: #{e.message}")
        $stderr.puts(e.backtrace&.first(5)&.join("\n") || "  (no backtrace)")
      end

      @thread
    end

    # Stop the web server gracefully.
    def stop
      @server&.stop(true)
      @thread&.join(5)
      File.delete(@socket_path) if @socket_path && File.exist?(@socket_path)
    end

    private

    # Configure the Portal BaseApp and all sub-apps with the MOP client
    # so they can issue driver requests for auth, git, and MR operations.
    def configure_portal_apps
      MudAdapter::Stdlib::Portal::BaseApp.mop_client = @client
      MudAdapter::Stdlib::Portal::BaseApp.session_handler = @session_handler
      MudAdapter::Stdlib::Portal::BaseApp.server_name_value = "MUD Driver"
      MudAdapter::Stdlib::Portal::BaseApp.area_loader = @area_loader
      MudAdapter::Stdlib::Portal::BaseApp.area_logger = @area_loader.logger
      MudAdapter::Stdlib::Portal::BaseApp.views_dir = MudAdapter::StdlibRuntime.views_dir
      if ENV['MUD_WORLD_PATH']
        MudAdapter::Stdlib::Portal::BaseApp.world_path_value = ENV['MUD_WORLD_PATH']
      end
    end

    # Build the Rack application stack with session middleware wrapping the
    # Portal::App Roda application.
    def build_rack_app
      secret = @session_secret
      portal_app = MudAdapter::Stdlib::Portal::App

      Rack::Builder.new do
        use Rack::Session::Cookie,
            key: "mud.session",
            secret: secret,
            same_site: :lax,
            httponly: true,
            path: "/"

        run portal_app
      end
    end

    # Generate a random session secret for cookie signing.
    def generate_session_secret
      require "securerandom"
      SecureRandom.hex(32)
    end
  end
end
