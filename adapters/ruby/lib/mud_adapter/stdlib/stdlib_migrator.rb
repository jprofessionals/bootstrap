# frozen_string_literal: true

require 'sequel'
require 'sequel/extensions/migration'

module MudAdapter
  module Stdlib
    # Runs the stdlib Sequel migrations against the given database URL.
    # Called when the driver sends a Configure message with the stdlib DB URL.
    module StdlibMigrator
      MIGRATIONS_DIR = File.expand_path('migrations', __dir__)

      # Connect to the stdlib database and run all pending migrations.
      # Idempotent — safe to call on every boot.
      def self.run!(db_url)
        db = Sequel.connect(db_url)
        Sequel::Migrator.run(db, MIGRATIONS_DIR)
        db.disconnect
      end
    end
  end
end
