# frozen_string_literal: true

require 'json'

module MudAdapter
  module Stdlib
    module Portal
      class PlayApp < BaseApp
        plugin :json

        route do |r|
          r.post 'start' do
            require_login!
            username = current_character || current_account

            session_id, output = session_handler.create_web_session(username)
            session['play_session_id'] = session_id

            response['Content-Type'] = 'application/json'
            { output: output }.to_json
          end

          r.post 'command' do
            require_login!
            play_session_id = session['play_session_id']
            halt_status(400) unless play_session_id

            body = r.body.read
            data = JSON.parse(body) rescue {}
            input = data['input'].to_s.strip
            halt_status(400) if input.empty?

            output = session_handler.execute_command(play_session_id, input)
            response['Content-Type'] = 'application/json'
            { output: output }.to_json
          end

          r.root do
            require_login!
            render_full(:play, server_name: server_name, character: session['character'])
          end
        end
      end
    end
  end
end
