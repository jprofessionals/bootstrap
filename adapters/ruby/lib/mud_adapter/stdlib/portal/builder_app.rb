# frozen_string_literal: true

require 'erb'
require 'json'
require 'fileutils'
require 'open3'
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
        @spa_builds = {}     # area_key => true (build completed)
        @user_apps = {}      # area_key => Rack app (from web_app block)

        class << self
          attr_reader :web_configs, :spa_builds, :user_apps
        end

        # rubocop:disable Metrics/BlockLength
        route do |r|
          # /builder/
          r.root do
            areas_index_page
          end

          # /builder/<namespace>/<area_with_branch>/...
          r.on String, String do |namespace, area_with_branch|
            # Redirect /builder/ns/area to /builder/ns/area/ so relative
            # asset paths (./assets/foo.js) resolve correctly.
            if r.remaining_path.empty?
              r.redirect "#{r.matched_path}/"
            end

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

            # Static files from web/ directory
            r.on 'web' do
              remaining = r.remaining_path.sub(%r{^/}, '')
              serve_static(work_path, remaining)
            end

            # Build logs API
            r.get 'api', 'logs' do
              limit = [(r.params['limit']&.to_i || 50), 200].min
              level = r.params['level'] || 'all'
              logs = area_logger&.recent(area_key, limit: limit, level: level) || []
              response['Content-Type'] = 'application/json'
              logs.to_json
            end

            # User Rack app (from web_app block) — 404 falls through.
            # In SPA mode, only forward /api/* requests to the Rack app;
            # all other paths are served by the SPA frontend.
            if config&.app_block
              remaining_path = r.remaining_path.empty? ? '/' : r.remaining_path
              serve_to_rack = !config.spa_mode? || remaining_path.start_with?('/api')

              if serve_to_rack
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
              end
            end

            # API routes from web_routes block (matched before SPA fallback)
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

            if config&.spa_mode?
              serve_spa(r, work_path, config, area, area_key)
            else
              # ERB mode: serve index.erb at root
              r.root do
                serve_erb(work_path, config, area)
              end

              # Serve static files (style.css etc.) at top level
              remaining = r.remaining_path.sub(%r{^/}, '')
              serve_static(work_path, remaining) unless remaining.empty?
            end
          end
        end
        # rubocop:enable Metrics/BlockLength

        private

        # Build or retrieve cached user Rack app from web_app block.
        def get_or_build_user_app(config, work_path, area_key)
          cached = self.class.user_apps[area_key]
          return cached if cached

          app = config.app_block.call(work_path)
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
        def load_web_config(work_path, area_key)
          cached = self.class.web_configs[area_key]
          current_gen = area_loader&.generation_for(area_key.split('@').first)

          if cached && cached[:generation] == current_gen
            return cached[:config]
          end

          # Invalidate SPA build cache on reload
          if cached && cached[:generation] != current_gen
            self.class.spa_builds.delete(area_key)
            self.class.user_apps.delete(area_key)
          end

          mud_web_path = File.join(work_path, 'mud_web.rb')
          config = MudAdapter::Stdlib::World::WebDataDSL.evaluate(mud_web_path)

          self.class.web_configs[area_key] = {
            config: config,
            generation: current_gen
          }

          config
        end

        # Render ERB mode: evaluate web_data block, render web/index.erb
        def serve_erb(work_path, config, area)
          erb_path = File.join(work_path, 'web', 'index.erb')
          unless File.exist?(erb_path)
            response.status = 404
            halt_body
          end

          locals = build_locals(config, area)

          erb = ERB.new(File.read(erb_path))
          b = binding
          locals.each { |k, v| b.local_variable_set(k, v) }

          response['Content-Type'] = 'text/html'
          erb.result(b)
        end

        # Build template locals by calling the web_data block.
        # Returns an empty hash if config or area is missing, so ERB templates
        # should handle missing locals gracefully (or the page renders with blanks).
        def build_locals(config, area)
          return {} unless config&.data_block

          # If area isn't loaded yet, provide a stub so ERB doesn't crash
          area ||= StubArea.new
          helpers = WebDataHelpers.new(self)
          config.data_block.call(area, helpers)
        rescue StandardError => e
          { error: "web_data error: #{e.message}" }
        end

        # Serve static files from web/ directory
        def serve_static(work_path, relative_path)
          return if relative_path.empty?

          # Path traversal protection
          full = File.expand_path(File.join(work_path, 'web', relative_path))
          web_dir = File.expand_path(File.join(work_path, 'web'))
          unless full.start_with?(web_dir) && File.file?(full)
            response.status = 404
            halt_body
          end

          content_type = guess_content_type(full)
          response['Content-Type'] = content_type
          File.read(full)
        end

        # Serve API routes defined in web_routes block
        def serve_api_routes(r, config, area)
          unless config&.routes_block
            response.status = 404
            halt_body
          end

          result = config.routes_block.call(r, area, session)
          if result.is_a?(Hash)
            response['Content-Type'] = 'application/json'
            result.to_json
          else
            result
          end
        end

        # SPA mode: build if needed, serve from temp dir
        def serve_spa(r, work_path, config, area, area_key)
          build_dir = spa_build_dir(area_key)

          # Build if not already built (run_spa_build returns true if it ran)
          unless self.class.spa_builds[area_key]
            if run_spa_build(work_path, build_dir, area_key)
              self.class.spa_builds[area_key] = true
            end
          end

          dist_dir = File.join(build_dir, 'dist')
          unless Dir.exist?(dist_dir)
            response.status = 503
            response['Content-Type'] = 'text/plain'
            halt_body('SPA build not available')
          end

          # Serve files from dist/
          remaining = r.remaining_path.sub(%r{^/}, '')
          remaining = 'index.html' if remaining.empty?

          full = File.expand_path(File.join(dist_dir, remaining))
          unless full.start_with?(File.expand_path(dist_dir)) && File.file?(full)
            # SPA fallback: serve index.html for client-side routing
            full = File.join(dist_dir, 'index.html')
            unless File.file?(full)
              response.status = 404
              halt_body
            end
          end

          response['Content-Type'] = guess_content_type(full)
          File.read(full)
        end

        def spa_build_dir(area_key)
          # area_key is "namespace/name" or "namespace/name@branch"
          File.join('/tmp', 'mud-builder-cache', area_key.tr('/', '-'))
        end

        # Returns true if a build was attempted, false if no SPA source found.
        def run_spa_build(work_path, build_dir, area_key)
          src_dir = File.join(work_path, 'web', 'src')
          return false unless Dir.exist?(src_dir) && File.exist?(File.join(src_dir, 'package.json'))

          FileUtils.rm_rf(build_dir)
          FileUtils.mkdir_p(build_dir)
          # Copy all files including dotfiles, preserving directory structure
          FileUtils.cp_r(Dir.glob(File.join(src_dir, '{*,.*}'), File::FNM_DOTMATCH)
                            .reject { |f| %w[. ..].include?(File.basename(f)) }, build_dir)

          # npm install
          output, status = Open3.capture2e('npm', 'install', chdir: build_dir)
          unless status.success?
            $stderr.puts "[builder] SPA npm install failed in #{build_dir}"
            area_logger&.log(area_key, :error, :spa_build, "npm install failed:\n#{output}")
            return true
          end
          area_logger&.log(area_key, :info, :spa_build, "npm install succeeded")

          # Compute the URL base path for this area's SPA
          base_url = "/builder/#{area_key}/"

          # vite build with correct base URL for asset paths
          output, status = Open3.capture2e(
            { 'MUD_BASE_URL' => base_url },
            'npx', 'vite', 'build', '--base', base_url,
            chdir: build_dir
          )
          unless status.success?
            $stderr.puts "[builder] SPA vite build failed in #{build_dir}"
            area_logger&.log(area_key, :error, :spa_build, "vite build failed:\n#{output}")
            return true
          end
          area_logger&.log(area_key, :info, :spa_build, "vite build succeeded")

          # Inject window.__MUD__ into dist/index.html for JS runtime
          inject_mud_global(build_dir, base_url)
          true
        end

        # Post-process dist/index.html to inject the MUD platform global.
        # This gives JS code access to the area's base URL at runtime.
        def inject_mud_global(build_dir, base_url)
          index_path = File.join(build_dir, 'dist', 'index.html')
          return unless File.exist?(index_path)

          html = File.read(index_path)
          mud_script = "<script>window.__MUD__={baseUrl:#{base_url.to_json}};</script>"

          # Inject after <head> tag
          unless html.include?('window.__MUD__')
            html = html.sub('<head>', "<head>\n  #{mud_script}")
            File.write(index_path, html)
          end
        end

        # Simple content-type guessing
        def guess_content_type(path)
          case File.extname(path).downcase
          when '.html', '.htm' then 'text/html'
          when '.css' then 'text/css'
          when '.js' then 'application/javascript'
          when '.json' then 'application/json'
          when '.png' then 'image/png'
          when '.jpg', '.jpeg' then 'image/jpeg'
          when '.gif' then 'image/gif'
          when '.svg' then 'image/svg+xml'
          when '.ico' then 'image/x-icon'
          when '.woff' then 'font/woff'
          when '.woff2' then 'font/woff2'
          when '.ttf' then 'font/ttf'
          else 'application/octet-stream'
          end
        end

        # List areas that have mud_web.rb for the index page
        def areas_index_page
          areas = []
          if area_loader
            area_loader.all_areas.each do |key, area|
              web_path = File.join(area.path, 'mud_web.rb')
              next unless File.exist?(web_path)

              areas << { key: key, name: area.name, namespace: area.namespace, path: area.path }
            end
          end

          render_view(:builder_index, server_name: server_name, page_title: 'Builder',
                                      areas: areas)
        end

        # Stub area returned when the real area isn't loaded yet.
        # Provides safe defaults so ERB templates don't crash.
        class StubArea
          def path;    '' end
          def rooms;   {} end
          def items;   {} end
          def npcs;    {} end
          def daemons; {} end
          def name;    'loading' end
          def namespace; '' end
        end

        # Helpers object passed to web_data blocks
        class WebDataHelpers
          def initialize(app)
            @app = app
          end

          def server_name
            @app.send(:server_name)
          end

          def total_players_online
            0 # TODO: wire to session count
          end
        end
      end
    end
  end
end
