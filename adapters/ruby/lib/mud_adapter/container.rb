# frozen_string_literal: true

module MUD
  module Container
    @registry = {}

    class << self
      def []=(key, val)
        @registry[key] = val
      end

      def [](key)
        @registry[key]
      end

      def delete(key)
        @registry.delete(key)
      end

      def key?(key)
        @registry.key?(key)
      end
    end
  end
end
