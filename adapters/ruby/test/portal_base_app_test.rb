# frozen_string_literal: true

# Test that portal BaseApp class-level attributes are properly inherited
# by all subclasses (AccountApp, EditorApp, GitApp, ReviewApp, PlayApp).
#
# Run with: ruby test/portal_base_app_test.rb
#   (from adapters/ruby/ directory)
#
# Background: BaseApp uses class-level instance variables via attr_writer
# + custom getters. Plain attr_accessor would NOT inherit values set on
# the parent class to subclasses, because Ruby class-level instance
# variables are per-class-object, not shared through inheritance.

ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)
require "bundler/setup"

require File.expand_path("../../../bootstrap/ruby/stdlib/portal/base_app", __dir__)
require File.expand_path("../../../bootstrap/ruby/stdlib/portal/account_app", __dir__)
require File.expand_path("../../../bootstrap/ruby/stdlib/portal/editor_app", __dir__)
require File.expand_path("../../../bootstrap/ruby/stdlib/portal/git_app", __dir__)
require File.expand_path("../../../bootstrap/ruby/stdlib/portal/review_app", __dir__)
require File.expand_path("../../../bootstrap/ruby/stdlib/portal/play_app", __dir__)

module PortalBaseAppTest
  FAILURES = []
  PASSES = []

  def self.assert(description, condition)
    if condition
      PASSES << description
    else
      FAILURES << description
      $stderr.puts "  FAIL: #{description}"
    end
  end

  def self.run
    puts "Running portal BaseApp inheritance tests..."

    sentinel_client = Object.new
    sentinel_name = "TestMUD"

    # Set values on BaseApp (simulates configure_portal_apps in WebServer)
    MudAdapter::Stdlib::Portal::BaseApp.mop_client = sentinel_client
    MudAdapter::Stdlib::Portal::BaseApp.server_name_value = sentinel_name

    subclasses = [
      MudAdapter::Stdlib::Portal::AccountApp,
      MudAdapter::Stdlib::Portal::EditorApp,
      MudAdapter::Stdlib::Portal::GitApp,
      MudAdapter::Stdlib::Portal::ReviewApp,
      MudAdapter::Stdlib::Portal::PlayApp
    ]

    # Test: each subclass should see mop_client set on BaseApp
    subclasses.each do |klass|
      short_name = klass.name.split("::").last

      assert(
        "#{short_name}.mop_client returns value set on BaseApp",
        klass.mop_client.equal?(sentinel_client)
      )

      assert(
        "#{short_name}.server_name_value returns value set on BaseApp",
        klass.server_name_value == sentinel_name
      )
    end

    # Test: BaseApp itself still works
    assert(
      "BaseApp.mop_client returns the set value",
      MudAdapter::Stdlib::Portal::BaseApp.mop_client.equal?(sentinel_client)
    )

    # Test: subclass can override without affecting parent or siblings
    override_client = Object.new
    MudAdapter::Stdlib::Portal::EditorApp.mop_client = override_client

    assert(
      "EditorApp.mop_client returns its own override",
      MudAdapter::Stdlib::Portal::EditorApp.mop_client.equal?(override_client)
    )

    assert(
      "BaseApp.mop_client is unaffected by EditorApp override",
      MudAdapter::Stdlib::Portal::BaseApp.mop_client.equal?(sentinel_client)
    )

    assert(
      "AccountApp.mop_client is unaffected by EditorApp override",
      MudAdapter::Stdlib::Portal::AccountApp.mop_client.equal?(sentinel_client)
    )

    # Report
    puts ""
    total = PASSES.size + FAILURES.size
    puts "#{total} assertions: #{PASSES.size} passed, #{FAILURES.size} failed"

    if FAILURES.any?
      puts "\nFailed:"
      FAILURES.each { |f| puts "  - #{f}" }
      exit 1
    else
      puts "All tests passed."
    end
  end
end

PortalBaseAppTest.run
