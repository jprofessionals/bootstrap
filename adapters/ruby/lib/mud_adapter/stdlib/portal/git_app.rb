# frozen_string_literal: true

require 'json'

module MudAdapter
  module Stdlib
    module Portal
      class GitApp < BaseApp
        # rubocop:disable Metrics/BlockLength
        route do |r|
          require_builder!

          r.root do
            git_dashboard_page
          end

          r.on 'api' do
            r.get 'repos' do
              list_repos
            end

            r.on 'repos', String, String do |namespace, name|
              check_repo_access!(namespace, name, :read_only)

              r.get 'status' do
                repo_status(namespace, name)
              end

              r.get 'log' do
                repo_log(namespace, name)
              end

              r.get 'diff' do
                repo_diff(namespace, name)
              end

              r.get 'branches' do
                list_branches(namespace, name)
              end

              r.post 'branches' do

                body = parse_json_body(r)
                create_branch(namespace, name, body['name'])
              end

              r.post 'checkout' do

                body = parse_json_body(r)
                checkout_branch(namespace, name, body['branch'])
              end

              r.post 'stage' do

                body = parse_json_body(r)
                stage_files(namespace, name, body['files'])
              end

              r.post 'unstage' do

                body = parse_json_body(r)
                unstage_files(namespace, name, body['files'])
              end

              r.post 'commit' do

                body = parse_json_body(r)
                commit_changes(namespace, name, body['message'])
              end

              r.post 'merge' do

                body = parse_json_body(r)
                merge_branch(namespace, name, body['branch'])
              end

              r.get 'tree' do
                browse_tree(namespace, name, r.params['path'] || '')
              end

              r.on 'merge-requests' do
                r.get true do
                  list_merge_requests(namespace, name)
                end

                r.post true do
  
                  body = parse_json_body(r)
                  create_merge_request(namespace, name, body)
                end

                r.on Integer do |mr_id|
                  r.get true do
                    show_merge_request(mr_id)
                  end

                  r.post 'approve' do
                    body = parse_json_body(r)
                    approve_merge_request(mr_id, body)
                  end

                  r.post 'reject' do
                    body = parse_json_body(r)
                    reject_merge_request(mr_id, body)
                  end

                  r.get 'diff' do
                    merge_request_diff(mr_id, namespace, name)
                  end

                  r.post 'merge' do
                    execute_merge_request(mr_id)
                  end
                end
              end
            end
          end
        end
        # rubocop:enable Metrics/BlockLength

        private

        def list_repos
          account = current_account
          # TODO: Send DriverRequest to list repos via MOP
          repo_names = mop_client&.send_driver_request('repo_list', { owner: account }) || []
          repos = repo_names.map { |name| "#{account}/#{name}" }
          { repos: repos }
        end

        def repo_status(namespace, name)
          branch = request.params['branch'] || default_branch(namespace, name)
          changes = mop_client&.send_driver_request('workspace_diff', {
            namespace: namespace, name: name, branch: branch
          }) || []
          { changes: changes }
        rescue MudAdapter::Client::DriverError
          { changes: [], error: 'workspace not available' }
        end

        def repo_log(namespace, name)
          limit = request.params['limit']&.to_i || 20
          branch = request.params['branch'] || default_branch(namespace, name)
          commits = mop_client&.send_driver_request('workspace_log', {
            namespace: namespace, name: name, branch: branch, limit: limit
          }) || []
          { commits: commits }
        rescue MudAdapter::Client::DriverError
          { commits: [], error: 'workspace not available' }
        end

        def repo_diff(namespace, name)
          branch = request.params['branch'] || default_branch(namespace, name)
          changes = mop_client&.send_driver_request('workspace_diff', {
            namespace: namespace, name: name, branch: branch
          }) || []
          { changes: changes }
        rescue MudAdapter::Client::DriverError
          { changes: [], error: 'workspace not available' }
        end

        def list_branches(namespace, name)
          branches = mop_client&.send_driver_request('workspace_branches', {
            namespace: namespace, name: name
          }) || []
          { branches: branches.map { |b| { name: b, current: false } } }
        rescue MudAdapter::Client::DriverError
          { branches: [], error: 'workspace not available' }
        end

        def create_branch(namespace, name, branch_name)
          halt_status(400) unless branch_name && !branch_name.empty?

          mop_client&.send_driver_request('workspace_create_branch', {
            namespace: namespace, name: name, branch_name: branch_name
          })

          response.status = 201
          { status: 'created', branch: branch_name }
        end

        def checkout_branch(namespace, name, branch_name)
          halt_status(400) unless branch_name && !branch_name.empty?

          mop_client&.send_driver_request('workspace_checkout_branch', {
            namespace: namespace, name: name, branch: branch_name
          })

          { status: 'ok', branch: branch_name }
        end

        def stage_files(_namespace, _name, _files)
          halt_status(501) # Not yet implemented via MOP
        end

        def unstage_files(_namespace, _name, _files)
          halt_status(501) # Not yet implemented via MOP
        end

        def commit_changes(namespace, name, message)
          halt_status(400) unless message && !message.empty?

          # TODO: Send DriverRequest to commit via MOP
          mop_client&.send_driver_request('workspace_commit', {
            namespace: namespace, name: name, author: current_account, message: message
          })
          trigger_reload(namespace, name)
          { status: 'committed', message: message }
        end

        def merge_branch(_namespace, _name, _branch_name)
          halt_status(501) # Not yet implemented via MOP
        end

        def browse_tree(_namespace, _name, _path)
          halt_status(501) # Not yet implemented via MOP
        end

        def list_merge_requests(namespace, name)
          mrs = mop_client&.send_driver_request('mr_list_all', {
            namespace: namespace, name: name
          }) || []
          { merge_requests: mrs }
        rescue MudAdapter::Client::DriverError
          { merge_requests: [], error: 'merge requests not available' }
        end

        def create_merge_request(namespace, name, body)
          halt_status(400) unless body&.dig('title')
          # TODO: Send DriverRequest to create MR via MOP
          result = mop_client&.send_driver_request('mr_create', {
            namespace: namespace, name: name, author: current_account,
            title: body['title'], description: body['description'],
            source_branch: body['source_branch'] || 'develop',
            target_branch: body['target_branch'] || 'main'
          }) || {}
          response.status = 201
          { merge_request: result }
        end

        def show_merge_request(mr_id)
          # TODO: Send DriverRequest to get MR via MOP
          mr = mop_client&.send_driver_request('mr_get', { id: mr_id })
          halt_status(404) unless mr
          { merge_request: mr }
        end

        def approve_merge_request(mr_id, body)
          # TODO: Send DriverRequest to approve MR via MOP
          result = mop_client&.send_driver_request('mr_approve', {
            id: mr_id, username: current_account, comment: body&.dig('comment')
          }) || {}
          { result: result }
        end

        def reject_merge_request(mr_id, body)
          # TODO: Send DriverRequest to reject MR via MOP
          mop_client&.send_driver_request('mr_reject', {
            id: mr_id, username: current_account, reason: body&.dig('reason')
          })
          { status: 'rejected' }
        end

        def execute_merge_request(mr_id)
          result = mop_client&.send_driver_request('mr_merge', { id: mr_id }) || {}
          { status: 'merged', merge_request: result }
        rescue MudAdapter::Client::DriverError => e
          response.status = 422
          { status: 'error', message: e.message }
        end

        def merge_request_diff(_mr_id, _namespace, _name)
          halt_status(501) # Not yet implemented via MOP
        end

        def default_branch(namespace, name)
          dev_path = File.join(world_path, namespace, "#{name}@dev")
          Dir.exist?(dev_path) ? 'develop' : 'main'
        end

        def active_merge_requests(namespace, name)
          # TODO: Send DriverRequest to list MRs via MOP
          mrs = mop_client&.send_driver_request('mr_list_all', {
            namespace: namespace, name: name, state: 'open'
          }) || []
          approved = mop_client&.send_driver_request('mr_list_all', {
            namespace: namespace, name: name, state: 'approved'
          }) || []
          (mrs + approved).group_by { |mr| mr[:source_branch] }
        rescue StandardError
          {}
        end

        def resolve_work_path(namespace, name)
          dev_path = File.join(world_path, namespace, "#{name}@dev")
          Dir.exist?(dev_path) ? dev_path : File.join(world_path, namespace, name)
        end

        def parse_json_body(request)
          return nil unless request.content_type&.include?('application/json')

          body = request.body.read
          JSON.parse(body)
        rescue JSON::ParserError
          nil
        end

        def git_dashboard_page
          account = current_account
          # TODO: Send DriverRequest to list repos via MOP
          repo_names = mop_client&.send_driver_request('repo_list', { owner: account }) || []
          repos = repo_names.map { |name| "#{account}/#{name}" }
          render_full(:git_dashboard, server_name: server_name, repos: repos)
        end

        def trigger_reload(namespace, name)
          work_path = resolve_work_path(namespace, name)
          mop_client&.send_driver_request('area_reload', {
            area_id: "#{namespace}/#{name}", path: work_path
          })
        rescue StandardError
          # Best-effort: don't fail the commit if reload fails
          nil
        end
      end
    end
  end
end
