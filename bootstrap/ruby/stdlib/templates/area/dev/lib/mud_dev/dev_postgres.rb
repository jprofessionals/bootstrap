module MudDev
  class DevPostgres
    attr_reader :area_name

    PG_IMAGE = 'postgres:16-alpine'.freeze
    PG_DATA_PATH = '/var/lib/postgresql/data'.freeze

    def initialize(area_name:)
      @area_name = area_name
      @container = nil
    end

    def volume_name = "mud-dev-#{@area_name}-pgdata"
    def container_name = "mud-dev-#{@area_name}-pg"
    def database_name = "mud_dev_#{@area_name}"

    def connection_config
      {
        adapter: :postgres,
        host: @host || '127.0.0.1',
        port: @port || 5432,
        user: 'postgres',
        password: 'postgres',
        database: database_name
      }
    end

    def start!(reset: false)
      require 'testcontainers'

      remove_volume! if reset
      ensure_volume!

      @container = Testcontainers::DockerContainer.new(
        PG_IMAGE,
        name: container_name,
        exposed_ports: { '5432/tcp' => {} },
        env: {
          'POSTGRES_USER' => 'postgres',
          'POSTGRES_PASSWORD' => 'postgres',
          'POSTGRES_DB' => database_name,
          'PGDATA' => "#{PG_DATA_PATH}/pgdata"
        }
      )
      @container.add_volume(volume_name, PG_DATA_PATH)
      @container.start
      @container.wait_for_tcp_port(5432)

      @host = @container.host
      @port = @container.mapped_port(5432)

      wait_for_postgres_ready!
      puts "PostgreSQL ready on #{@host}:#{@port} (database: #{database_name})"
    rescue StandardError => e
      raise "Failed to start PostgreSQL container. Is Docker running? Error: #{e.message}"
    end

    def stop!
      return unless @container

      @container.stop
      @container.remove
      @container = nil
      puts "PostgreSQL container stopped (volume #{volume_name} preserved)"
    end

    def connect
      require 'sequel'
      Sequel.connect(connection_config)
    end

    def remove_volume!
      require 'testcontainers'

      begin
        Docker::Container.get(container_name).tap do |c|
          begin
            c.stop
          rescue StandardError
            nil
          end
          c.remove(force: true)
        end
      rescue Docker::Error::NotFoundError
        # Container doesn't exist
      end

      begin
        Docker::Volume.get(volume_name).remove
        puts "Removed volume #{volume_name}"
      rescue Docker::Error::NotFoundError
        # Volume doesn't exist
      end
    end

    private

    def ensure_volume!
      Docker::Volume.get(volume_name)
    rescue Docker::Error::NotFoundError
      Docker::Volume.create(volume_name)
    end

    def wait_for_postgres_ready!
      require 'sequel'
      20.times do
        db = Sequel.connect(connection_config.merge(database: 'postgres'))
        db.fetch('SELECT 1').first
        db.disconnect
        ensure_database!
        return
      rescue Sequel::DatabaseConnectionError
        sleep 0.5
      end
      raise 'PostgreSQL did not become ready in time'
    end

    def ensure_database!
      require 'sequel'
      admin = Sequel.connect(connection_config.merge(database: 'postgres'))
      unless admin.fetch('SELECT 1 FROM pg_database WHERE datname = ?', database_name).any?
        admin.run("CREATE DATABASE #{admin.literal(database_name.to_sym)}")
      end
      admin.disconnect
    end
  end
end
