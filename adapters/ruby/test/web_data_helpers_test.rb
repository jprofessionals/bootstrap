# frozen_string_literal: true

ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)
require "bundler/setup"

require_relative "../lib/mud_adapter"

module WebDataHelpersTest
  FAILURES = []
  PASSES = []

  def self.assert(description, condition)
    if condition
      PASSES << description
    else
      FAILURES << description
      warn "  FAIL: #{description}"
    end
  end

  def self.run
    puts "Running web data helper tests..."

    area = Struct.new(:path, :rooms, :items, :npcs).new(
      "/tmp/world/alice/demo",
      [Object.new, Object.new],
      [Object.new],
      [Object.new, Object.new, Object.new]
    )

    session_handler = Object.new
    def session_handler.total_players_online
      7
    end

    helpers = MudAdapter::Stdlib::World::WebDataHelpers.new(
      server_name: "SpecMUD",
      session_handler: session_handler
    )

    dsl = MudAdapter::Stdlib::World::WebDataDSL.evaluate(
      File.expand_path("../../../bootstrap/ruby/stdlib/templates/area/mud_web.rb", __dir__)
    )
    data = dsl.data_block.call(area, helpers)

    assert("web_data receives helper-backed server name", data[:server_name] == "SpecMUD")
    assert("web_data receives helper-backed online count", data[:players_online] == 7)
    assert("web_data still reads area data", data[:room_count] == 2)

    total = PASSES.size + FAILURES.size
    puts "#{total} assertions: #{PASSES.size} passed, #{FAILURES.size} failed"

    exit 1 if FAILURES.any?
  end
end

WebDataHelpersTest.run
