# frozen_string_literal: true

require 'roda'
require 'erb'

module MudAdapter
  module Stdlib
    module Portal
      class BaseApp < Roda
        plugin :json

        class << self
          attr_writer :mop_client, :session_handler, :server_name_value, :views_dir, :world_path_value, :area_loader, :area_logger

          # Class-level instance variables are NOT inherited by subclasses.
          # These getters walk the ancestor chain so that values set on BaseApp
          # are visible to AccountApp, EditorApp, etc.

          def mop_client
            return @mop_client if instance_variable_defined?(:@mop_client)

            superclass.mop_client if superclass.respond_to?(:mop_client)
          end

          def session_handler
            return @session_handler if instance_variable_defined?(:@session_handler)

            superclass.session_handler if superclass.respond_to?(:session_handler)
          end

          def server_name_value
            return @server_name_value if instance_variable_defined?(:@server_name_value)

            superclass.server_name_value if superclass.respond_to?(:server_name_value)
          end

          def views_dir
            return @views_dir if instance_variable_defined?(:@views_dir)

            superclass.views_dir if superclass.respond_to?(:views_dir)
          end

          def world_path_value
            return @world_path_value if instance_variable_defined?(:@world_path_value)

            superclass.world_path_value if superclass.respond_to?(:world_path_value)
          end

          def area_loader
            return @area_loader if instance_variable_defined?(:@area_loader)

            superclass.area_loader if superclass.respond_to?(:area_loader)
          end

          def area_logger
            return @area_logger if instance_variable_defined?(:@area_logger)

            superclass.area_logger if superclass.respond_to?(:area_logger)
          end
        end

        private

        def views_dir
          self.class.views_dir || File.expand_path('views', __dir__)
        end

        def require_login!
          unless session['account'] && session['session_token']
            session.clear
            request.redirect '/account/login'
            return
          end

          # Validate session token with the driver via MOP
          if mop_client
            valid = mop_client.send_driver_request('session_validate', {
              account: session['account'],
              token: session['session_token']
            })
            unless valid
              session.clear
              request.redirect '/account/login'
            end
          end
        end

        def require_builder!
          require_login!
          return unless session['role'] == 'player'

          response.status = 403
          halt_body
        end

        def current_account
          session['account']
        end

        def current_character
          session['character']
        end

        def current_role
          session['role']
        end

        def mop_client
          self.class.mop_client
        end

        def session_handler
          self.class.session_handler
        end

        def server_name
          self.class.server_name_value || 'MUD'
        end

        def world_path
          self.class.world_path_value || 'world'
        end

        def area_loader
          self.class.area_loader
        end

        def area_logger
          self.class.area_logger
        end

        def render_view(template, locals = {})
          path = File.join(views_dir, "#{template}.erb")
          erb = ERB.new(File.read(path))
          b = binding
          locals.each { |k, v| b.local_variable_set(k, v) }
          content = erb.result(b)

          layout_path = File.join(views_dir, 'layout.erb')
          layout = ERB.new(File.read(layout_path))
          b2 = binding
          b2.local_variable_set(:content, content)
          b2.local_variable_set(:server_name, locals[:server_name] || '')
          b2.local_variable_set(:page_title, locals[:page_title] || '')
          b2.local_variable_set(:extra_head, locals[:extra_head])
          layout.result(b2)
        end

        def render_full(template, locals = {})
          path = File.join(views_dir, "#{template}.erb")
          erb = ERB.new(File.read(path))
          b = binding
          locals.each { |k, v| b.local_variable_set(k, v) }
          erb.result(b)
        end

        def check_repo_access!(namespace, name, level = :read_only)
          allowed = MudAdapter::Stdlib::System::AccessControl.repo_access_allowed?(
            username: current_account,
            namespace: namespace,
            name: name,
            level: level.to_s
          )
          return if allowed

          response.status = 403
          halt_body
        end

        def halt_status(code)
          response.status = code
          response.write('')
          request.halt
        end

        # Halt with a body string. Uses response.write + request.halt (no args)
        # so that response.finish returns a proper [status, headers, body] triplet.
        # Calling request.halt(string) directly throws the string as the halt value,
        # which breaks when this app is mounted as a sub-app via r.run — the session
        # middleware receives a string instead of a Rack triplet and gets nil headers.
        def halt_body(body = '')
          response.write(body)
          request.halt
        end
      end
    end
  end
end
