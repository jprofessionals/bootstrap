# frozen_string_literal: true

require "yaml"

module MudAdapter
  module Stdlib
    module System
      module AccessControl
        module_function

        def repo_access_allowed?(username:, namespace:, name:, level:)
          return false if username.nil? || username.empty?

          normalized = normalize_level(level)
          global_policy = policy_for(namespace, name)
          repo_policy = load_repo_policy(namespace, name)
          role = role_for(username)

          return true if system_admin?(role)
          return true if repo_owner?(repo_policy, username)
          return true if explicitly_allowed_user?(repo_policy, username, normalized)
          return true if explicitly_allowed_role?(repo_policy, role, normalized)
          return true if explicitly_allowed_user?(global_policy, username, normalized)
          return true if explicitly_allowed_role?(global_policy, role, normalized)
          false
        end

        def normalize_level(level)
          value = level.to_s
          value == "read_write" ? "read_write" : "read_only"
        end

        def policy_for(namespace, name)
          config = load_policy_config
          repo_key = "#{namespace}/#{name}"
          repo_policy = config.fetch("repos", {})[repo_key]
          return repo_policy if repo_policy.is_a?(Hash)

          if namespace == "system"
            config.fetch("system_repos", {})
          else
            config.fetch("builder_repos", {})
          end
        end

        def role_for(username)
          db = MUD::Container["database.stdlib"]
          return nil unless db

          row = db[:players].where(id: username).select(:role).first
          row && row[:role].to_s
        rescue StandardError
          nil
        end

        def system_admin?(role)
          role == "admin"
        end

        def explicitly_allowed_user?(policy, username, level)
          users = Array(policy.fetch("users", []))
          return true if users.include?(username)

          access = policy.fetch("user_levels", {})[username]
          access_allows?(access, level)
        end

        def explicitly_allowed_role?(policy, role, level)
          return false if role.nil? || role.empty?

          roles = Array(policy.fetch("roles", []))
          return true if roles.include?(role)

          access = policy.fetch("role_levels", {})[role]
          access_allows?(access, level)
        end

        def access_allows?(access, requested_level)
          normalized = normalize_level(access)
          return false if access.nil?

          requested_level == "read_only" || normalized == "read_write"
        end

        def load_policy_config
          policy_path = File.join(stdlib_root, "config", "repo_policy.yml")
          return default_policy unless File.file?(policy_path)

          data = YAML.safe_load(File.read(policy_path)) || {}
          data.is_a?(Hash) ? data : default_policy
        rescue StandardError
          default_policy
        end

        def load_repo_policy(namespace, name)
          policy_path = File.join(git_root, namespace, "#{name}.git.policy.yml")
          return default_repo_policy(namespace) unless File.file?(policy_path)

          data = YAML.safe_load(File.read(policy_path)) || {}
          return default_repo_policy(namespace) unless data.is_a?(Hash)

          {
            "owner" => data["owner"] || namespace,
            "users" => Array(data["users"]).map(&:to_s),
            "roles" => Array(data["roles"]).map(&:to_s),
            "user_levels" => stringify_map(data["user_levels"]),
            "role_levels" => stringify_map(data["role_levels"])
          }
        rescue StandardError
          default_repo_policy(namespace)
        end

        def stringify_map(value)
          return {} unless value.is_a?(Hash)

          value.each_with_object({}) do |(key, access), acc|
            acc[key.to_s] = access.to_s
          end
        end

        def repo_owner?(policy, username)
          policy.fetch("owner", nil) == username
        end

        def git_root
          ENV["MUD_GIT_PATH"] || "data/git-server"
        end

        def stdlib_root
          MudAdapter::StdlibRuntime.root_path
        end

        def default_policy
          {
            "system_repos" => {
              "role_levels" => {
                "admin" => "read_write"
              }
            },
            "builder_repos" => {
              "role_levels" => {
                "admin" => "read_write"
              }
            },
            "repos" => {}
          }
        end

        def default_repo_policy(namespace)
          {
            "owner" => namespace,
            "users" => [],
            "roles" => [],
            "user_levels" => {},
            "role_levels" => {}
          }
        end
      end
    end
  end
end
