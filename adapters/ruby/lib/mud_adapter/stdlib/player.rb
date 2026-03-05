# frozen_string_literal: true

module MudAdapter
  module Stdlib
    class Player
      attr_reader :id, :name

      def initialize(id:, name:)
        @id = id
        @name = name
        @output_buffer = []
      end

      def send_output(text)
        @output_buffer << text
      end

      def flush_output
        buffer = @output_buffer.dup
        @output_buffer.clear
        buffer
      end
    end
  end
end
