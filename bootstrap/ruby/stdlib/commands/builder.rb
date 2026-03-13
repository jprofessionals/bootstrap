# frozen_string_literal: true

require 'yaml'

module MudAdapter
  module Stdlib
    module Commands
      class Builder
        # @param client [MudAdapter::Client] MOP client for driver requests
        # @param username [String] current builder's username
        def initialize(client:, username:)
          @client = client
          @username = username
        end

        def handle(command)
          case command.verb
          when :@repo
            handle_repo(command.args)
          when :@project
            handle_project(command.args)
          when :@commit
            handle_commit(command.args)
          when :@log
            handle_log(command.args)
          when :@diff
            handle_diff(command.args)
          when :@reload
            handle_reload(command.args)
          when :@arealog
            handle_arealog(command.args)
          when :@driverlog
            handle_driverlog(command.args)
          when :@review
            handle_review(command.args)
          else
            "Unknown builder command: #{command.verb}"
          end
        end

        private

        def handle_repo(args)
          sub = args[0]
          case sub
          when 'new'
            name = args[1]
            return 'Usage: @repo new <name>' unless name

            # TODO: Send DriverRequest to create repo via MOP
            @client.send_driver_request('repo_create', { owner: @username, name: name })
            "Repository '#{name}' created."
          when 'list'
            # TODO: Send DriverRequest to list repos via MOP
            repos = @client.send_driver_request('repo_list', { owner: @username })
            return 'No repos found.' if repos.empty?

            repos.join("\n")
          when 'grant'
            name = args[1]
            target_user = args[2]
            level = args[3]
            return 'Usage: @repo grant <repo> <user> <read_only|read_write>' unless name && target_user && level

            # TODO: Send DriverRequest to grant access via MOP
            @client.send_driver_request('repo_grant', {
              owner: @username, name: name, target_user: target_user, level: level
            })
            "Access granted to #{target_user} on #{name} (#{level})."
          when 'revoke'
            name = args[1]
            target_user = args[2]
            return 'Usage: @repo revoke <repo> <user>' unless name && target_user

            # TODO: Send DriverRequest to revoke access via MOP
            @client.send_driver_request('repo_revoke', {
              owner: @username, name: name, target_user: target_user
            })
            "Access revoked for #{target_user} on #{name}."
          else
            'Usage: @repo <new|list|grant|revoke>'
          end
        end

        def handle_project(args)
          case args[0]
          when 'new'      then project_new(args[1])
          when 'list'     then project_list
          else 'Usage: @project <new|list>'
          end
        end

        def project_new(name)
          return 'Usage: @project new <name>' unless name

          # TODO: Send DriverRequest to create repo + checkout via MOP
          @client.send_driver_request('project_create', { owner: @username, name: name })
          "Project '#{name}' created and checked out."
        end

        def project_list
          # TODO: Send DriverRequest to list repos via MOP
          repos = @client.send_driver_request('repo_list', { owner: @username })
          return 'No projects found.' if repos.empty?

          repos.map { |r| "  #{r}" }.join("\n")
        end

        def handle_commit(args)
          message = args[0]
          return 'Usage: @commit "message"' unless message

          'Commit requires an active project context.'
        end

        def handle_log(_args)
          'Log requires an active project context.'
        end

        def handle_diff(_args)
          'Diff requires an active project context.'
        end

        def handle_reload(_args)
          'Reload requires an active project context.'
        end

        def handle_arealog(args)
          area_key = args[0]
          return 'Usage: @arealog <area_key> [source] [severity]' unless area_key

          # TODO: Send DriverRequest to get area log entries via MOP
          'Area log not yet available via MOP.'
        end

        def handle_driverlog(args)
          # TODO: Send DriverRequest to get driver log entries via MOP
          'Driver log not yet available via MOP.'
        end

        def handle_review(args)
          case args[0]
          when 'list'    then review_list(args[1])
          when 'new'     then review_new(args[1], args[2..])
          when 'show'    then review_show(args[1])
          when 'approve' then review_approve(args[1], args[2..])
          when 'reject'  then review_reject(args[1], args[2..])
          when 'merge'   then review_merge(args[1])
          when 'close'   then review_close(args[1])
          else 'Usage: @review <list|new|show|approve|reject|merge|close>'
          end
        end

        def review_list(area_name)
          return 'Usage: @review list <area_name>' unless area_name

          # TODO: Send DriverRequest to list MRs via MOP
          mrs = @client.send_driver_request('mr_list', {
            username: @username, area_name: area_name, state: 'open'
          })
          return 'No open merge requests.' if mrs.empty?

          mrs.map { |mr| "##{mr[:id]} [#{mr[:state]}] #{mr[:title]} (by #{mr[:author]})" }.join("\n")
        end

        def review_new(area_name, title_parts)
          title = title_parts&.join(' ')
          return 'Usage: @review new <area_name> <title>' if !area_name || title.nil? || title.empty?

          # TODO: Send DriverRequest to create MR via MOP
          mr = @client.send_driver_request('mr_create', {
            username: @username, area_name: area_name, author: @username, title: title
          })
          if mr[:success] == false
            mr[:error]
          else
            "Merge request ##{mr[:id]} created: #{mr[:title]}"
          end
        end

        def review_show(id_str)
          return 'Usage: @review show <id>' unless id_str

          # TODO: Send DriverRequest to get MR via MOP
          mr = @client.send_driver_request('mr_get', { id: id_str.to_i })
          return 'Merge request not found.' unless mr

          lines = []
          lines << "MR ##{mr[:id]}: #{mr[:title]}"
          lines << "State: #{mr[:state]} | Author: #{mr[:author]}"
          lines << "Area: #{mr[:namespace]}/#{mr[:area_name]}"
          lines << "Approvals: #{mr[:approvals_count]}"
          mr[:approvals]&.each { |a| lines << "  - #{a[:approver]}: #{a[:comment] || '(no comment)'}" }
          lines.join("\n")
        end

        def review_approve(id_str, comment_parts)
          return 'Usage: @review approve <id> [comment]' unless id_str

          comment = comment_parts&.join(' ')
          comment = nil if comment&.empty?
          # TODO: Send DriverRequest to approve MR via MOP
          result = @client.send_driver_request('mr_approve', {
            id: id_str.to_i, username: @username, comment: comment
          })
          result[:success] ? "Approved MR ##{id_str}." : result[:error]
        end

        def review_reject(id_str, reason_parts)
          return 'Usage: @review reject <id> [reason]' unless id_str

          reason = reason_parts&.join(' ')
          reason = nil if reason&.empty?
          # TODO: Send DriverRequest to reject MR via MOP
          @client.send_driver_request('mr_reject', {
            id: id_str.to_i, username: @username, reason: reason
          })
          "Rejected MR ##{id_str}."
        end

        def review_merge(id_str)
          return 'Usage: @review merge <id>' unless id_str

          # TODO: Send DriverRequest to merge MR via MOP
          result = @client.send_driver_request('mr_merge', { id: id_str.to_i })
          result[:success] ? "Merged MR ##{id_str}." : result[:error]
        end

        def review_close(id_str)
          return 'Usage: @review close <id>' unless id_str

          # TODO: Send DriverRequest to close MR via MOP
          @client.send_driver_request('mr_close', { id: id_str.to_i })
          "Closed MR ##{id_str}."
        end
      end
    end
  end
end
