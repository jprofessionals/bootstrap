require 'securerandom'
require_relative 'web_data_dsl'
require_relative 'spa_api_app'
require_relative 'dev_postgres'

module MudDev
  class DevServer
    attr_reader :web_config, :area_stub, :dev_postgres

    AreaStub = Struct.new(:path, :rooms, :items, :npcs, :daemons, :name, :namespace) do
      def system? = false
    end

    def initialize(area_path:, enable_db: false, reset_db: false)
      @area_path = area_path
      @enable_db = enable_db
      @reset_db = reset_db

      web_file = File.join(area_path, 'mud_web.rb')
      @web_config = WebDataDSL.evaluate(web_file)

      @area_stub = AreaStub.new(
        path: area_path, rooms: {}, items: {}, npcs: {},
        daemons: {}, name: File.basename(area_path), namespace: 'dev'
      )

      @dev_postgres = DevPostgres.new(area_name: File.basename(area_path)) if enable_db
    end

    def build_api_app
      SpaApiApp.build(
        routes_block: @web_config.routes_block,
        area: @area_stub,
        session_secret: SecureRandom.hex(64)
      )
    end

    def start(api_port: 4000)
      require 'async'
      require 'async/http/server'
      require 'async/http/endpoint'
      require 'protocol/rack'

      setup_database! if @enable_db

      api_app = build_api_app.freeze.app
      middleware = Protocol::Rack::Adapter.new(api_app)
      endpoint = Async::HTTP::Endpoint.parse("http://127.0.0.1:#{api_port}")

      puts "Starting API server on http://127.0.0.1:#{api_port}"

      vite_pid = start_vite_dev_server
      trap('INT') do
        shutdown!(vite_pid)
        exit
      end
      trap('TERM') do
        shutdown!(vite_pid)
        exit
      end

      Async do
        server = Async::HTTP::Server.new(middleware, endpoint)
        server.run
      end
    ensure
      shutdown!(vite_pid)
    end

    private

    def setup_database!
      @dev_postgres.start!(reset: @reset_db)
      @area_db = @dev_postgres.connect

      migrations_dir = File.join(@area_path, 'db', 'migrations')
      return unless Dir.exist?(migrations_dir)

      require 'sequel/extensions/migration'
      Sequel::Migrator.run(@area_db, migrations_dir)
      puts "Migrations applied from #{migrations_dir}"
    end

    def shutdown!(vite_pid)
      Process.kill(:TERM, vite_pid) if vite_pid
      @area_db&.disconnect
      @dev_postgres&.stop!
    rescue StandardError
      nil
    end

    def start_vite_dev_server
      src_dir = File.join(@area_path, 'web', 'src')
      return nil unless Dir.exist?(src_dir)

      pid = spawn('npx', 'vite', 'dev', '--host', '127.0.0.1', chdir: src_dir)
      puts "Vite dev server starting (PID #{pid})..."
      puts 'Ready at http://localhost:5173'
      pid
    end
  end
end
