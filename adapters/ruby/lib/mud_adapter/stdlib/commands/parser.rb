# frozen_string_literal: true

require_relative 'command'

module MudAdapter
  module Stdlib
    module Commands
      module Parser
        TEXT_COMMANDS = %i[say @commit].freeze

        def self.parse(input)
          input = input.strip
          return nil if input.empty?

          if input.start_with?('@')
            parse_builder_command(input)
          else
            parse_game_command(input)
          end
        end

        def self.parse_builder_command(input)
          parts = input.split(' ', 2)
          verb = parts[0]

          if verb == '@commit'
            msg = parts[1]&.gsub(/\A["']|["']\z/, '')
            args = msg ? [msg] : []
          else
            args = parts[1]&.split || []
          end

          Command.new(verb: verb, args: args)
        end

        def self.parse_game_command(input)
          parts = input.split(' ', 2)
          verb = parts[0].downcase

          args = if TEXT_COMMANDS.include?(verb.to_sym)
                   parts[1] ? [parts[1]] : []
                 else
                   parts[1]&.split || []
                 end

          Command.new(verb: verb, args: args)
        end

        private_class_method :parse_builder_command, :parse_game_command
      end
    end
  end
end
