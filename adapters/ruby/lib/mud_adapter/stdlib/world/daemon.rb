# frozen_string_literal: true

require_relative 'game_object'

module MudAdapter
  module Stdlib
    module World
      class Daemon < GameObject
        DEFAULT_INTERVAL = 60

        class << self
          def interval(value = nil)
            if value
              @interval = value
            else
              @interval || DEFAULT_INTERVAL
            end
          end

          def inherited(subclass)
            super
            subclass.instance_variable_set(:@interval, @interval)
          end
        end

        def interval
          self.class.interval
        end

        def on_tick
          # default no-op
        end
      end
    end
  end
end
