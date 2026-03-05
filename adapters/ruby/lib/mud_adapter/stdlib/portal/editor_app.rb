# frozen_string_literal: true

require 'json'

module MudAdapter
  module Stdlib
    module Portal
      class EditorApp < BaseApp
        plugin :all_verbs
        plugin :slash_path_empty

        # rubocop:disable Metrics/BlockLength
        route do |r|
          require_builder!

          r.post 'api/pull' do
            json_body = parse_json_body(r)
            repo_param = r.params['repo'] || json_body&.dig('repo')
            halt_status(400) unless repo_param

            namespace, name = repo_param.split('/', 2)
            halt_status(400) unless namespace && name

            check_repo_access!(namespace, name, :read_write)

            # Ensure workspace checkout exists, then pull latest
            ensure_checkout(namespace, name)
            branch = default_branch(namespace, name)
            mop_client&.send_driver_request('workspace_pull', {
              namespace: namespace, name: name, branch: branch
            })
            work_path = resolve_work_path(namespace, name)

            { branch: branch, files: list_files(work_path)[:files] }
          end

          r.root do
            editor_page
          end
        end
        # rubocop:enable Metrics/BlockLength

        private

        # Ensure both production and @dev checkouts exist.
        def ensure_checkout(namespace, name)
          dev_path = File.join(world_path, namespace, "#{name}@dev")
          prod_path = File.join(world_path, namespace, name)
          return if Dir.exist?(dev_path) && Dir.exist?(prod_path)

          mop_client&.send_driver_request('workspace_checkout', {
            namespace: namespace, name: name
          })
        end

        def resolve_work_path(namespace, name)
          # TODO: Send DriverRequest to get workspace paths via MOP
          # For now, use a convention-based path
          dev_path = File.join(world_path, namespace, "#{name}@dev")
          Dir.exist?(dev_path) ? dev_path : File.join(world_path, namespace, name)
        end

        def default_branch(namespace, name)
          dev_path = File.join(world_path, namespace, "#{name}@dev")
          Dir.exist?(dev_path) ? 'develop' : 'main'
        end

        def list_files(work_path)
          files = []
          Dir.glob(File.join(work_path, '**', '*')).each do |path|
            next unless File.file?(path)
            next if path.include?('/.git/')

            relative = path.sub("#{work_path}/", '')
            files << { path: relative, size: File.size(path) }
          end
          { files: files.sort_by { |f| f[:path] } }
        end

        def parse_json_body(request)
          return nil unless request.content_type&.include?('application/json')

          body = request.body.read
          JSON.parse(body)
        rescue JSON::ParserError
          nil
        end

        def editor_page
          account = session['account']
          repos = list_builder_repos(account)
          render_full(:editor, server_name: server_name, repos: repos)
        end

        def list_builder_repos(account)
          # TODO: Send DriverRequest to list repos via MOP
          repos = mop_client&.send_driver_request('repo_list', { owner: account })
          (repos || []).map { |name| "#{account}/#{name}" }
        end
      end
    end
  end
end
