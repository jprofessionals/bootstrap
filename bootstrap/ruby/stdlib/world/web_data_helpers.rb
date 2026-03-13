# frozen_string_literal: true

module MudAdapter
  module Stdlib
    module World
      class WebDataHelpers
        def initialize(server_name:, session_handler:)
          @server_name = server_name
          @session_handler = session_handler
        end

        def server_name
          @server_name
        end

        def total_players_online
          @session_handler&.total_players_online || 0
        end
      end
    end
  end
end
