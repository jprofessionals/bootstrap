# frozen_string_literal: true

module MudAdapter
  module Stdlib
    module Commands
      class Command
        attr_reader :verb, :args

        def initialize(verb:, args:)
          @verb = verb.to_sym
          @args = Array(args).map(&:to_s)
        end

        def builder_command?
          verb.to_s.start_with?('@')
        end
      end
    end
  end
end
