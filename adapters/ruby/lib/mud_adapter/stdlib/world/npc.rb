# frozen_string_literal: true

require_relative 'game_object'

module MudAdapter
  module Stdlib
    module World
      class NPC < GameObject
        class << self
          def location(value = nil)
            if value
              @location = value
            else
              @location
            end
          end

          def inherited(subclass)
            super
            subclass.instance_variable_set(:@location, @location)
          end
        end

        def location
          self.class.location
        end

        def on_talk(player)
          # default no-op
        end
      end
    end
  end
end
