# frozen_string_literal: true

require 'fileutils'

module MudAdapter
  # Loads area data from disk using the stdlib Area class.
  #
  # Each area is loaded by evaluating its Ruby source files through the
  # stdlib Area class, which handles the mud_aliases.rb, mud_loader.rb,
  # and directory scanning automatically.
  class AreaLoader
    def initialize(client)
      @client = client
      @areas = {} # "namespace/name" => Area instance
      @generation = Hash.new(0)
      @logger = MudAdapter::AreaLogger.new(client)
    end

    attr_reader :logger

    # Load an area from disk using the stdlib Area class.
    #
    # area_id is a Hash with "namespace" and "name" keys (matching the
    # Rust AreaId struct serialization).
    def load_area(area_id, path, db_url: nil)
      key = area_key(area_id)
      @logger.register_area(key, path)
      @logger.log(key, :info, :reload_start, "Loading area #{key}")

      error_count = 0

      begin
        area = MudAdapter::Stdlib::World::Area.new(path)
        area.on_file_error = ->(file, err) {
          error_count += 1
          bt = err.backtrace&.first(10)&.join("\n")
          @logger.log(key, :error, :file_load, "#{err.class}: #{err.message} in #{file}", backtrace: bt)
        }
        area.load!
        @areas[key] = area
        @generation[key] += 1

        connect_area_db(key, path, db_url)

        if error_count > 0
          @logger.log(key, :warn, :reload_end, "Loaded with #{error_count} error(s)")
        else
          @logger.log(key, :info, :reload_end, "Loaded successfully")
        end

        build_spa_if_needed(key, path)

        @client.send_message(
          "type" => "area_loaded",
          "area_id" => area_id
        )
      rescue StandardError => e
        bt = e.backtrace&.first(10)&.join("\n")
        @logger.log(key, :error, :reload_end, "#{e.class}: #{e.message}", backtrace: bt)
        @client.send_message(
          "type" => "area_error",
          "area_id" => area_id,
          "error" => "#{e.class}: #{e.message}"
        )
      end
    end

    # Reload a previously loaded area by clearing its data and reloading.
    def reload_area(area_id, path, db_url: nil)
      key = area_key(area_id)
      existing = @areas[key]

      unless existing
        load_area(area_id, path, db_url: db_url)
        return
      end

      @logger.register_area(key, path)
      @logger.log(key, :info, :reload_start, "Reloading area #{key}")

      error_count = 0

      begin
        existing.on_file_error = ->(file, err) {
          error_count += 1
          bt = err.backtrace&.first(10)&.join("\n")
          @logger.log(key, :error, :file_load, "#{err.class}: #{err.message} in #{file}", backtrace: bt)
        }
        existing.reload!
        @generation[key] += 1

        connect_area_db(key, path, db_url)

        if error_count > 0
          @logger.log(key, :warn, :reload_end, "Reloaded with #{error_count} error(s)")
        else
          @logger.log(key, :info, :reload_end, "Reloaded successfully")
        end

        build_spa_if_needed(key, path)

        @client.send_message(
          "type" => "area_loaded",
          "area_id" => area_id
        )
      rescue StandardError => e
        bt = e.backtrace&.first(10)&.join("\n")
        @logger.log(key, :error, :reload_end, "#{e.class}: #{e.message}", backtrace: bt)
        @client.send_message(
          "type" => "area_error",
          "area_id" => area_id,
          "error" => "#{e.class}: #{e.message}"
        )
      end
    end

    # Unload an area and free its data.
    def unload_area(area_id)
      key = area_key(area_id)
      @areas.delete(key)

      # Disconnect area database
      db_key = "database.#{key}"
      if MUD::Container.key?(db_key)
        MUD::Container[db_key]&.disconnect rescue nil
        MUD::Container.delete(db_key)
      end

      @logger.unregister_area(key)
    end

    # Return loaded Area instance (for inspection/debugging).
    def get_area(area_id)
      @areas[area_key(area_id)]
    end

    # Find a Room instance by area key and room name.
    # Returns the Room instance or nil.
    def find_room(area_key, room_name)
      area = @areas[area_key]
      return nil unless area

      area.rooms[room_name]
    end

    # Return a flat hash of all rooms across all areas.
    # Keys are "area_key/room_name", values are Room instances.
    def all_rooms
      result = {}
      @areas.each do |area_key, area|
        area.rooms.each do |room_name, room|
          result["#{area_key}/#{room_name}"] = room
        end
      end
      result
    end

    # Find which area key a room belongs to.
    # Returns the area_key string or nil.
    def area_for_room(room_name)
      @areas.each do |area_key, area|
        return area_key if area.rooms.key?(room_name)
      end
      nil
    end

    # Return the first room of the first loaded area, or nil.
    def first_room
      @areas.each do |area_key, area|
        first_name = area.rooms.keys.first
        next unless first_name

        return { key: first_name, area_key: area_key }
      end
      nil
    end

    # Check if any areas are loaded.
    def any_areas?
      !@areas.empty?
    end

    # Return the generation counter for a given area, or nil if not loaded.
    def generation_for(area_key)
      @areas[area_key]&.generation
    end

    # Return all loaded areas as a hash of "namespace/name" => Area.
    def all_areas
      @areas.dup
    end

    private

    # Build a string key from an area_id hash.
    def area_key(area_id)
      "#{area_id["namespace"]}/#{area_id["name"]}"
    end

    def connect_area_db(area_key, path, db_url)
      return unless db_url

      require 'sequel'

      db_registry_key = "database.#{area_key}"

      # Disconnect existing connection on reload
      if MUD::Container.key?(db_registry_key)
        MUD::Container[db_registry_key]&.disconnect rescue nil
      end

      db = Sequel.connect(db_url)
      MUD::Container[db_registry_key] = db

      # Auto-run migrations
      migrations_dir = File.join(path, 'db', 'migrations')
      if Dir.exist?(migrations_dir)
        Sequel::Migrator.run(db, migrations_dir)
        count = Dir.glob(File.join(migrations_dir, '*.rb')).size
        @logger.log(area_key, :info, :migration, "Ran migrations (#{count} files)")
      end
    rescue StandardError => e
      bt = e.backtrace&.first(10)&.join("\n")
      @logger.log(area_key, :error, :migration, "#{e.class}: #{e.message}", backtrace: bt)
    end

    # Build the SPA if the area uses web_mode :spa.
    # Runs npm install + vite build eagerly so the first web request is fast.
    # Builds both the main area and the @dev branch checkout if it exists.
    def build_spa_if_needed(area_key, path)
      mud_web_path = File.join(path, 'mud_web.rb')
      return unless File.exist?(mud_web_path)

      config = MudAdapter::Stdlib::World::WebDataDSL.evaluate(mud_web_path)
      return unless config&.spa_mode?

      # Build main area
      run_spa_build_for(area_key, path)

      # Also build @dev checkout if it exists
      dev_path = "#{path}@dev"
      if Dir.exist?(dev_path)
        dev_key = "#{area_key}@dev"
        run_spa_build_for(dev_key, dev_path)
      end
    end

    def run_spa_build_for(area_key, path)
      require 'open3'

      src_dir = File.join(path, 'web', 'src')
      return unless Dir.exist?(src_dir) && File.exist?(File.join(src_dir, 'package.json'))

      build_dir = File.join('/tmp', 'mud-builder-cache', area_key.tr('/', '-'))
      FileUtils.rm_rf(build_dir)
      FileUtils.mkdir_p(build_dir)
      FileUtils.cp_r(Dir.glob(File.join(src_dir, '{*,.*}'), File::FNM_DOTMATCH)
                        .reject { |f| %w[. ..].include?(File.basename(f)) }, build_dir)

      # npm install
      output, status = Open3.capture2e('npm', 'install', chdir: build_dir)
      unless status.success?
        @logger.log(area_key, :error, :spa_build, "npm install failed:\n#{output}")
        return
      end
      @logger.log(area_key, :info, :spa_build, "npm install succeeded")

      # vite build with correct base URL
      base_url = "/builder/#{area_key}/"
      output, status = Open3.capture2e(
        { 'MUD_BASE_URL' => base_url },
        'npx', 'vite', 'build', '--base', base_url,
        chdir: build_dir
      )
      unless status.success?
        @logger.log(area_key, :error, :spa_build, "vite build failed:\n#{output}")
        return
      end
      @logger.log(area_key, :info, :spa_build, "vite build succeeded")

      # Inject window.__MUD__
      index_path = File.join(build_dir, 'dist', 'index.html')
      if File.exist?(index_path)
        html = File.read(index_path)
        mud_script = "<script>window.__MUD__={baseUrl:#{base_url.to_json}};</script>"
        unless html.include?('window.__MUD__')
          html = html.sub('<head>', "<head>\n  #{mud_script}")
          File.write(index_path, html)
        end
      end

      # Mark as built so BuilderApp doesn't rebuild
      MudAdapter::Stdlib::Portal::BuilderApp.spa_builds[area_key] = true
    rescue StandardError => e
      bt = e.backtrace&.first(10)&.join("\n")
      @logger.log(area_key, :error, :spa_build, "#{e.class}: #{e.message}", backtrace: bt)
    end

    # Log an info message via the MOP client.
    def log_info(message)
      @client.send_message(
        "type" => "log",
        "level" => "info",
        "message" => message,
        "area" => nil
      )
    end
  end
end
