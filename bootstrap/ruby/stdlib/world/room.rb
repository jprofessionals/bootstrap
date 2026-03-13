# frozen_string_literal: true

require_relative 'game_object'

module MudAdapter
  module Stdlib
    module World
      class Room < GameObject
        class << self
          def exit(direction, to:)
            @exits ||= {}
            @exits[direction] = to
          end

          def exits
            @exits ||= {}
          end

          def inherited(subclass)
            super
            subclass.instance_variable_set(:@exits, @exits&.dup || {})
          end
        end

        def exits
          self.class.exits
        end

        def exit_directions
          exits.keys
        end

        def has_exit?(direction)
          exits.key?(direction)
        end

        def on_enter(player)
          # default no-op, subclasses override
        end
      end
    end
  end
end
