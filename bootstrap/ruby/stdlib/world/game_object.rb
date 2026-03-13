# frozen_string_literal: true

module MudAdapter
  module Stdlib
    module World
      class GameObject
        class << self
          def title(value = nil)
            if value
              @title = value
            else
              @title
            end
          end

          def description(value = nil)
            if value
              @description = value
            else
              @description
            end
          end

          def inherited(subclass)
            super
            subclass.instance_variable_set(:@title, @title)
            subclass.instance_variable_set(:@description, @description)
          end
        end

        def title
          self.class.title
        end

        def description
          self.class.description
        end
      end
    end
  end
end
