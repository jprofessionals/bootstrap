# frozen_string_literal: true

module MudAdapter
  module StdlibRuntime
    module_function

    SHARED_FILES = %w[
      stdlib_migrator.rb
      player.rb
      system/access_control.rb
      web/rack_app.rb
      world/web_data_helpers.rb
    ].freeze

    WORLD_FILES = %w[
      world/game_object.rb
      world/room.rb
      world/item.rb
      world/npc.rb
      world/daemon.rb
      world/area.rb
      world/review_policy.rb
      commands/command.rb
      commands/parser.rb
      commands/builder.rb
    ].freeze

    PORTAL_FILES = %w[
      portal/base_app.rb
      portal/account_app.rb
      portal/play_app.rb
      portal/editor_app.rb
      portal/git_app.rb
      portal/review_app.rb
      portal/builder_app.rb
      portal/app.rb
    ].freeze

    def load!
      root = resolve_root
      load_shared(root)
      load_world(root)
      load_portal(root)
      @current_root = root
    end

    def reload!(subsystem = :all)
      root = resolve_root
      case subsystem.to_sym
      when :world
        load_world(root)
      when :portal
        load_portal(root)
      else
        load_shared(root)
        load_world(root)
        load_portal(root)
      end
      @current_root = root
    end

    def refresh_from_world!
      root = resolve_root
      if @current_root != root
        load!
      end
    end

    def views_dir
      File.join(resolve_root, "portal", "views")
    end

    def root_path
      resolve_root
    end

    def current_root
      @current_root || resolve_root
    end

    def builtin_root
      File.expand_path("../../../../bootstrap/ruby/stdlib", __dir__)
    end

    def world_root
      world_path = ENV["MUD_WORLD_PATH"]
      return nil if world_path.nil? || world_path.empty?

      candidate = File.join(world_path, "system", "stdlib")
      File.directory?(candidate) ? candidate : nil
    end

    def resolve_root
      world_root || builtin_root
    end

    def load_shared(root)
      load_files(root, SHARED_FILES)
    end

    def load_world(root)
      load_files(root, WORLD_FILES)
    end

    def load_portal(root)
      load_files(root, PORTAL_FILES)
    end

    def load_files(root, files)
      previous_verbose = $VERBOSE
      $VERBOSE = nil
      files.each do |relative|
        load File.join(root, relative)
      end
    ensure
      $VERBOSE = previous_verbose
    end
  end
end
