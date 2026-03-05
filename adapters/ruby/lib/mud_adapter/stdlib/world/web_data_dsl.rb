# frozen_string_literal: true

module MudAdapter
  module Stdlib
    module World
      class WebDataDSL
        attr_reader :data_block, :routes_block, :app_block

        def initialize
          @data_block = nil
          @routes_block = nil
          @app_block = nil
          @mode = :template
        end

        def web_mode(mode)
          @mode = mode
        end

        def spa_mode?
          @mode == :spa
        end

        def web_data(&block)
          @data_block = block
        end

        def web_routes(&block)
          @routes_block = block
        end

        def web_app(&block)
          @app_block = block
        end

        # Ignore unknown DSL methods so area mud_web.rb files don't crash the server
        def method_missing(name, *args, &block)
          $stderr.puts "[web_data_dsl] unknown directive '#{name}' ignored"
        end

        def respond_to_missing?(name, include_private = false)
          true
        end

        def self.evaluate(path)
          return nil unless File.exist?(path)

          # Clear the RackApp subclass tracker before evaluation
          if defined?(MUD::Stdlib::Web::RackApp)
            MUD::Stdlib::Web::RackApp.recent_subclass = nil
          end

          dsl = new
          content = File.read(path)
          dsl.instance_eval(content, path)

          # Auto-detect RackApp subclass defined in the file
          if defined?(MUD::Stdlib::Web::RackApp) &&
             MUD::Stdlib::Web::RackApp.recent_subclass &&
             dsl.app_block.nil?
            app_class = MUD::Stdlib::Web::RackApp.recent_subclass
            dsl.instance_variable_set(:@app_block, ->(_work_path) { app_class })
          end

          dsl
        end
      end
    end
  end
end
