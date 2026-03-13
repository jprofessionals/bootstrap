# frozen_string_literal: true

ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)
require "bundler/setup"
require "fileutils"
require "tmpdir"
require "yaml"

require_relative "../lib/mud_adapter/container"
require_relative "../lib/mud_adapter/stdlib_runtime"
require File.expand_path("../../../bootstrap/ruby/stdlib/system/access_control", __dir__)

class FakeDataset
  def initialize(roles)
    @roles = roles
    @lookup_id = nil
  end

  def where(id:)
    @lookup_id = id
    self
  end

  def select(*)
    self
  end

  def first
    role = @roles[@lookup_id]
    role ? { role: role } : nil
  end
end

class FakeDb
  def initialize(roles)
    @roles = roles
  end

  def [](_table)
    FakeDataset.new(@roles)
  end
end

module AccessControlTest
  FAILURES = []
  PASSES = []

  module_function

  def assert(description, condition)
    if condition
      PASSES << description
    else
      FAILURES << description
      $stderr.puts "  FAIL: #{description}"
    end
  end

  def run
    puts "Running access control tests..."

    Dir.mktmpdir("mud-acl") do |dir|
      setup_policy(dir)
      setup_repo_policy(dir)
      setup_role_db
      with_temp_env("MUD_GIT_PATH", File.join(dir, "git")) do
        with_stdlib_root(dir) do
          run_assertions
        end
      end
    end

    puts ""
    total = PASSES.size + FAILURES.size
    puts "#{total} assertions: #{PASSES.size} passed, #{FAILURES.size} failed"
    exit(1) if FAILURES.any?
  end

  def setup_policy(root)
    config_dir = File.join(root, "config")
    FileUtils.mkdir_p(config_dir)
    policy = {
      "system_repos" => {
        "role_levels" => {
          "admin" => "read_write"
        }
      },
      "builder_repos" => {
        "role_levels" => {
          "builder" => "read_only"
        }
      },
      "repos" => {
        "builders/alpha" => {
          "role_levels" => {
            "builder" => "read_write"
          }
        },
        "system/stdlib" => {
          "users" => ["ops"]
        }
      }
    }
    File.write(File.join(config_dir, "repo_policy.yml"), YAML.dump(policy))
  end

  def setup_repo_policy(root)
    git_root = File.join(root, "git")
    FileUtils.mkdir_p(File.join(git_root, "builders"))
    File.write(
      File.join(git_root, "builders", "alpha.git.policy.yml"),
      YAML.dump(
        "owner" => "builders",
        "user_levels" => {
          "alice" => "read_write"
        }
      )
    )
    File.write(
      File.join(git_root, "builders", "village.git.policy.yml"),
      YAML.dump(
        "owner" => "alice",
        "user_levels" => {
          "bob" => "read_only"
        }
      )
    )
  end

  def setup_role_db
    MUD::Container["database.stdlib"] = FakeDb.new(
      "alice" => "builder",
      "bob" => "builder",
      "admin_user" => "admin",
      "ops" => "player"
    )
  end

  def with_temp_env(key, value)
    old = ENV[key]
    ENV[key] = value
    yield
  ensure
    ENV[key] = old
  end

  def with_stdlib_root(root)
    runtime = MudAdapter::StdlibRuntime.singleton_class
    original = MudAdapter::StdlibRuntime.method(:root_path)
    runtime.send(:define_method, :root_path) { root }
    yield
  ensure
    runtime.send(:define_method, :root_path, original)
  end

  def run_assertions
    acl = MudAdapter::Stdlib::System::AccessControl

    assert(
      "admin role can write system repo without ACL file",
      acl.repo_access_allowed?(
        username: "admin_user",
        namespace: "system",
        name: "stdlib",
        level: "read_write"
      )
    )

    assert(
      "repo policy user allow grants access without ACL file",
      acl.repo_access_allowed?(
        username: "ops",
        namespace: "system",
        name: "stdlib",
        level: "read_only"
      )
    )

    assert(
      "repo policy role override can grant builder write access",
      acl.repo_access_allowed?(
        username: "alice",
        namespace: "builders",
        name: "alpha",
        level: "read_write"
      )
    )

    assert(
      "repo owner still gets write access from repo policy",
      acl.repo_access_allowed?(
        username: "alice",
        namespace: "builders",
        name: "village",
        level: "read_write"
      )
    )

    assert(
      "repo policy still grants explicit collaborator read access",
      acl.repo_access_allowed?(
        username: "bob",
        namespace: "builders",
        name: "village",
        level: "read_only"
      )
    )

    assert(
      "builder cannot write repo without policy override or write ACL",
      !acl.repo_access_allowed?(
        username: "bob",
        namespace: "builders",
        name: "village",
        level: "read_write"
      )
    )
  end
end

AccessControlTest.run
