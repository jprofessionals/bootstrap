# frozen_string_literal: true

require_relative 'game_object'

module MudAdapter
  module Stdlib
    module World
      class Item < GameObject
        class << self
          def portable(value = nil)
            if value.nil?
              @portable || false
            else
              @portable = value
            end
          end

          def inherited(subclass)
            super
            subclass.instance_variable_set(:@portable, @portable)
          end
        end

        def portable?
          self.class.portable
        end

        def on_use(player, target)
          # default no-op
        end
      end
    end
  end
end
