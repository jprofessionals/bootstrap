# frozen_string_literal: true

require 'yaml'

module MudAdapter
  module Stdlib
    module World
      class Area
        attr_reader :path, :owner, :rooms, :items, :npcs, :daemons
        attr_accessor :namespace, :name, :on_reload, :on_file_error, :generation

        def initialize(path)
          @path = path
          @rooms = {}
          @items = {}
          @npcs = {}
          @daemons = {}
          load_meta!
          @name = File.basename(@path)
          @namespace = nil
          @on_file_error = nil
        end

        def system?
          @system == true
        end

        def load!
          load_aliases!
          load_loader_config!
          load_directories!
        end

        def reload!
          @rooms.clear
          @items.clear
          @npcs.clear
          @daemons.clear
          load!
          @on_reload&.call(self)
        end

        def reload_file(relative_path)
          full_path = File.join(@path, relative_path)
          load_single_file(full_path)
        end

        private

        def load_meta!
          meta_path = File.join(@path, '.meta.yml')
          if File.exist?(meta_path)
            meta = YAML.safe_load_file(meta_path)
            @owner = meta['owner']
            @system = meta['system'] == true
          else
            @owner = nil
            @system = false
          end
        end

        def load_aliases!
          alias_path = File.join(@path, 'mud_aliases.rb')
          load alias_path if File.exist?(alias_path)
        end

        def load_loader_config!
          loader_path = File.join(@path, 'mud_loader.rb')
          @directory_mappings = if File.exist?(loader_path)
                                  LoaderDSL.evaluate(loader_path)
                                else
                                  default_directory_mappings
                                end
        end

        def default_directory_mappings
          [
            { directory: 'rooms', type: MudAdapter::Stdlib::World::Room },
            { directory: 'items', type: MudAdapter::Stdlib::World::Item },
            { directory: 'npcs', type: MudAdapter::Stdlib::World::NPC },
            { directory: 'daemons', type: MudAdapter::Stdlib::World::Daemon }
          ]
        end

        def load_directories!
          @directory_mappings.each do |mapping|
            registry = registry_for(mapping[:type])
            load_directory(mapping[:directory], mapping[:type], registry)
          end
        end

        def registry_for(type)
          if type <= MudAdapter::Stdlib::World::Room
            @rooms
          elsif type <= MudAdapter::Stdlib::World::Item
            @items
          elsif type <= MudAdapter::Stdlib::World::NPC
            @npcs
          elsif type <= MudAdapter::Stdlib::World::Daemon
            @daemons
          else
            raise ArgumentError, "Unknown game object type: #{type}"
          end
        end

        def load_directory(subdir, base_class, registry)
          dir = File.join(@path, subdir)
          return unless Dir.exist?(dir)

          Dir.glob(File.join(dir, '*.rb')).each do |file|
            load_single_file(file, base_class, registry)
          end
        end

        def load_single_file(file, base_class = nil, registry = nil)
          begin
            load file
          rescue StandardError, SyntaxError => e
            @on_file_error&.call(file, e)
            return
          end

          return unless base_class && registry

          name = File.basename(file, '.rb')
          class_name = name.split('_').map(&:capitalize).join
          begin
            klass = Object.const_get(class_name)
            registry[name] = klass.new if klass < base_class
          rescue NameError
            # class not defined in this file
          end
        end
      end

      class LoaderDSL
        attr_reader :mappings

        def initialize
          @mappings = []
        end

        def directory(name, type:)
          @mappings << { directory: name, type: type }
        end

        def self.evaluate(path)
          dsl = new
          content = File.read(path)
          dsl.instance_eval(content, path)
          dsl.mappings
        end

        def loader(&)
          instance_eval(&)
        end
      end
    end
  end
end
