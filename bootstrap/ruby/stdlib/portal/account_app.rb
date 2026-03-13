# frozen_string_literal: true

require 'json'

module MudAdapter
  module Stdlib
    module Portal
      class AccountApp < BaseApp
        # rubocop:disable Metrics/BlockLength
        route do |r|
          # Internal session info endpoint used by the Rust HTTP server
          # to validate auth for AI proxy endpoints.
          r.get 'api', 'whoami' do
            if session['account'] && session['session_token']
              response['Content-Type'] = 'application/json'
              JSON.generate({
                'player_id' => session['account'],
                'role' => session['role'] || 'player'
              })
            else
              response.status = 401
              response['Content-Type'] = 'application/json'
              JSON.generate({ 'error' => 'not authenticated' })
            end
          end

          r.get 'login' do
            render_view(:login, server_name: server_name, page_title: 'Login', error: nil)
          end

          r.post 'login' do
            handle_login(r)
          end

          r.get 'register' do
            render_view(:register, server_name: server_name, page_title: 'Register', error: nil)
          end

          r.post 'register' do
            handle_register(r)
          end

          r.post 'logout' do
            # TODO: Send DriverRequest to destroy session via MOP
            mop_client&.send_driver_request('session_destroy', { token: session['session_token'] })
            session.clear
            r.redirect '/account/login'
          end

          require_login!

          r.get 'characters', 'new' do
            render_view(:new_character, server_name: server_name, page_title: 'New Character', error: nil)
          end

          r.post 'characters', 'new' do
            handle_new_character(r)
          end

          r.get 'characters', String do |char_name|
            character_detail(char_name)
          end

          r.get 'characters' do
            characters_page
          end

          r.post 'characters/switch' do
            handle_switch_character(r)
          end
        end
        # rubocop:enable Metrics/BlockLength

        private

        def handle_register(request)
          username = request.params['username']&.strip.to_s
          password = request.params['password'].to_s
          character = request.params['character']&.strip.to_s

          if username.empty? || password.empty? || character.empty?
            return render_view(:register, server_name: server_name, page_title: 'Register',
                                          error: 'All fields are required')
          end

          # TODO: Send DriverRequest to check if user exists via MOP
          existing = mop_client&.send_driver_request('player_find', { username: username })
          if existing
            return render_view(:register, server_name: server_name, page_title: 'Register',
                                          error: 'Username already taken')
          end

          # TODO: Send DriverRequest to create account via MOP
          mop_client&.send_driver_request('player_create', {
            username: username, password: password, character: character
          })
          start_session(username, character, 'builder')
          request.redirect '/account/characters'
        end

        def start_session(account, character, role)
          session['account'] = account
          session['character'] = character
          session['role'] = role
          # TODO: Send DriverRequest to create session token via MOP
          token = mop_client&.send_driver_request('session_create', { account: account })
          session['session_token'] = token
        end

        def handle_login(request)
          username = request.params['username']
          password = request.params['password']

          # TODO: Send DriverRequest to authenticate via MOP
          result = mop_client&.send_driver_request('player_authenticate', {
            username: username, password: password
          })

          if result && result['success']
            data = result['data']
            start_session(username, data['active_character'], data['role'])

            request.redirect '/account/characters'
          else
            render_view(:login, server_name: server_name, page_title: 'Login', error: 'Invalid credentials')
          end
        end

        def handle_switch_character(request)
          character_name = request.params['character']
          account = session['account']

          # TODO: Send DriverRequest to switch character via MOP
          result = mop_client&.send_driver_request('player_switch_character', {
            account: account, character: character_name
          })
          session['character'] = character_name if result

          request.redirect '/account/characters'
        end

        def handle_new_character(request)
          name = request.params['name']&.strip.to_s

          if name.empty?
            return render_view(:new_character, server_name: server_name, page_title: 'New Character',
                                               error: 'Character name is required')
          end

          account = session['account']
          # TODO: Send DriverRequest to get player data via MOP
          data = mop_client&.send_driver_request('player_find', { username: account })
          existing = (data&.dig('characters') || []).any? { |c| c['name'] == name }

          if existing
            return render_view(:new_character, server_name: server_name, page_title: 'New Character',
                                               error: 'Character name already taken')
          end

          # TODO: Send DriverRequest to add character via MOP
          mop_client&.send_driver_request('player_add_character', { account: account, name: name })
          request.redirect '/account/characters'
        end

        def characters_page
          account = session['account']
          # TODO: Send DriverRequest to get player data via MOP
          data = mop_client&.send_driver_request('player_find', { username: account }) || {}
          characters = data['characters'] || []
          active = data['active_character']

          render_view(:characters, server_name: server_name, page_title: 'Characters',
                                   characters: characters, active: active, role: session['role'])
        end

        def character_detail(char_name, error: nil)
          account = session['account']
          # TODO: Send DriverRequest to get player data via MOP
          data = mop_client&.send_driver_request('player_find', { username: account }) || {}
          characters = data['characters'] || []
          character = characters.find { |c| c['name'] == char_name }
          halt_status(404) unless character

          active = data['active_character']
          render_view(:character_detail, server_name: server_name, page_title: char_name,
                                         character: character, active: active, error: error)
        end
      end
    end
  end
end
