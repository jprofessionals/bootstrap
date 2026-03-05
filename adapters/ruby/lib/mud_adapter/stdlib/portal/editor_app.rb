# frozen_string_literal: true

require 'cgi'
require 'json'
require 'fileutils'

module MudAdapter
  module Stdlib
    module Portal
      class EditorApp < BaseApp
        plugin :all_verbs
        plugin :slash_path_empty
        MAX_FILE_SIZE = 1_048_576

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

          r.on 'api/files' do
            json_body = parse_json_body(r)
            repo_param = r.params['repo'] || json_body&.dig('repo')
            halt_status(400) unless repo_param

            namespace, name = repo_param.split('/', 2)
            halt_status(400) unless namespace && name

            check_repo_access!(namespace, name, :read_only)

            ensure_checkout(namespace, name)
            work_path = resolve_work_path(namespace, name)
            halt_status(404) unless Dir.exist?(work_path)

            r.get true do
              list_files(work_path)
            end

            remaining = CGI.unescape(r.remaining_path.sub(%r{^/}, ''))
            halt_status(400) if remaining.empty?

            safe_path = resolve_safe_path(work_path, remaining)
            halt_status(403) unless safe_path

            r.get do
              read_file(safe_path)
            end

            r.put do
              write_file(safe_path, json_body&.dig('content'))
            end

            r.post do
              create_file(safe_path, json_body&.dig('content'))
            end

            r.delete do
              delete_file(safe_path)
            end
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

        def read_file(path)
          halt_status(404) unless File.exist?(path)
          { content: File.read(path), path: path }
        end

        def write_file(path, content)
          halt_status(400) unless content
          halt_status(413) if content.bytesize > MAX_FILE_SIZE

          FileUtils.mkdir_p(File.dirname(path))
          File.write(path, content)
          { status: 'ok' }
        end

        def create_file(path, content)
          halt_status(400) unless content
          halt_status(413) if content.bytesize > MAX_FILE_SIZE
          halt_status(409) if File.exist?(path)

          FileUtils.mkdir_p(File.dirname(path))
          File.write(path, content)
          response.status = 201
          { status: 'created' }
        end

        def delete_file(path)
          halt_status(404) unless File.exist?(path)
          File.delete(path)
          { status: 'deleted' }
        end

        def resolve_safe_path(work_path, relative_path)
          full = File.expand_path(File.join(work_path, relative_path))
          return nil unless full.start_with?(File.expand_path(work_path))
          return nil if full.include?('/.git/')

          full
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
