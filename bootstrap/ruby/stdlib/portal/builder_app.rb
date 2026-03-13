# frozen_string_literal: true

require 'json'
require_relative '../world/web_data_dsl'

module MudAdapter
  module Stdlib
    module Portal
      class BuilderApp < BaseApp
        plugin :all_verbs
        plugin :slash_path_empty

        # Cache parsed WebDataDSL configs keyed by "namespace/name[@branch]"
        # with generation tracking for hot-reload.
        @web_configs = {}    # area_key => { config: WebDataDSL, generation: Integer }
        @user_apps = {}      # area_key => Rack app (from web_app block)

        class << self
          attr_reader :web_configs, :user_apps
        end

        route do |r|
          # /builder/
          r.root do
            response.status = 302
            response['Location'] = '/project/'
            ''
          end

          # /builder/<namespace>/<area_with_branch>/...
          r.on String, String do |namespace, area_with_branch|
            area_name, branch = parse_area_branch(area_with_branch)

            # Access control: non-main branches require login + repo access
            if branch
              require_login!
              check_repo_access!(namespace, area_name, :read_only)
            end

            work_path = resolve_builder_path(namespace, area_name, branch)
            unless work_path && Dir.exist?(work_path)
              response.status = 404
              halt_body
            end

            area_key = branch ? "#{namespace}/#{area_name}@#{branch}" : "#{namespace}/#{area_name}"
            config = load_web_config(work_path, area_key)
            area = area_loader&.get_area({ "namespace" => namespace, "name" => area_name })

            # Build logs API
            r.get 'api', 'logs' do
              limit = [(r.params['limit']&.to_i || 50), 200].min
              level = r.params['level'] || 'all'
              logs = area_logger&.recent(area_key, limit: limit, level: level) || []
              response['Content-Type'] = 'application/json'
              logs.to_json
            end

            # User Rack app (from web_app block) — only forward /api/* requests.
            if config&.app_block
              remaining_path = r.remaining_path.empty? ? '/' : r.remaining_path
              if remaining_path.start_with?('/api')
                begin
                  user_app = get_or_build_user_app(config, work_path, area_key)
                  env = r.env.dup
                  env['SCRIPT_NAME'] = r.matched_path
                  env['PATH_INFO'] = remaining_path
                  rack_status, rack_headers, rack_body = user_app.call(env)
                  unless rack_status == 404
                    response.status = rack_status
                    rack_headers.each { |k, v| response[k] = v }
                    rack_body.each { |chunk| response.write(chunk) }
                    rack_body.close if rack_body.respond_to?(:close)
                    request.halt
                  end
                rescue StandardError => e
                  $stderr.puts "[builder] web_app error: #{e.class}: #{e.message}"
                  $stderr.puts e.backtrace&.first(10)&.join("\n")
                  # Clear cached app so next request re-evaluates
                  self.class.user_apps.delete(area_key)
                  response.status = 500
                  response['Content-Type'] = 'application/json'
                  response.write({ error: "#{e.class}: #{e.message}" }.to_json)
                  request.halt
                end
              end
            end

            # API routes from web_routes block
            if config&.routes_block
              begin
                result = config.routes_block.call(r, area, session)
                if result.is_a?(Hash)
                  response['Content-Type'] = 'application/json'
                  next result.to_json
                elsif result
                  next result
                end
              rescue StandardError => e
                $stderr.puts "[builder] web_routes error: #{e.class}: #{e.message}"
                $stderr.puts e.backtrace&.first(10)&.join("\n")
                response.status = 500
                response['Content-Type'] = 'application/json'
                next { error: "#{e.class}: #{e.message}" }.to_json
              end
            end

            # Everything else (SPA, static, templates) is handled by the Rust driver.
          end
        end

        private

        # Build or retrieve cached user Rack app from web_app block.
        def get_or_build_user_app(config, work_path, area_key)
          cached = self.class.user_apps[area_key]
          return cached if cached

          app = config.app_block.call(work_path)

          # Wire area database into RackApp subclasses before freezing
          if defined?(MUD::Stdlib::Web::RackApp) && app.is_a?(Class) && app < MUD::Stdlib::Web::RackApp
            db_key = "database.#{area_key.split('@').first}"
            app.opts[:area_db] = MUD::Container[db_key] if MUD::Container.key?(db_key)
            app = app.app  # freeze into a Rack-callable app
          end

          self.class.user_apps[area_key] = app
          app
        end

        # Parse "test@dev" → ["test", "dev"], "test" → ["test", nil]
        def parse_area_branch(area_with_branch)
          if area_with_branch.include?('@')
            parts = area_with_branch.split('@', 2)
            [parts[0], parts[1]]
          else
            [area_with_branch, nil]
          end
        end

        # Resolve the working directory path for builder content.
        # Main branch uses the production checkout; named branches use @branch.
        def resolve_builder_path(namespace, area_name, branch)
          if branch
            File.join(world_path, namespace, "#{area_name}@#{branch}")
          else
            File.join(world_path, namespace, area_name)
          end
        end

        # Load (or re-load if stale) the WebDataDSL config for an area.
        # For @dev branches, use file mtime as the cache key since the area
        # generation only tracks the main branch.
        def load_web_config(work_path, area_key)
          cached = self.class.web_configs[area_key]
          mud_web_path = File.join(work_path, 'mud_web.rb')

          # Compute a cache version: area generation + file mtime
          current_gen = area_loader&.generation_for(area_key.split('@').first)
          file_mtime = File.exist?(mud_web_path) ? File.mtime(mud_web_path).to_f : nil
          cache_version = [current_gen, file_mtime]

          if cached && cached[:version] == cache_version
            return cached[:config]
          end

          # Invalidate user app cache on reload
          if cached
            self.class.user_apps.delete(area_key)
          end

          config = MudAdapter::Stdlib::World::WebDataDSL.evaluate(mud_web_path)

          self.class.web_configs[area_key] = {
            config: config,
            version: cache_version
          }

          config
        end
      end
    end
  end
end
