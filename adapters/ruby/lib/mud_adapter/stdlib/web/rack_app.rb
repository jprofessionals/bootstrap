# frozen_string_literal: true

require 'roda'
require 'json'

module MudAdapter
  module Stdlib
    module Web
      class RackApp < Roda
        plugin :json
        plugin :all_verbs

        # Track subclasses defined during WebDataDSL evaluation.
        @recent_subclass = nil

        class << self
          attr_accessor :recent_subclass
        end

        def self.inherited(subclass)
          super
          MudAdapter::Stdlib::Web::RackApp.recent_subclass = subclass
        end

        # Access the area's Sequel database from route blocks.
        def area_db
          opts[:area_db]
        end
      end
    end
  end
end
