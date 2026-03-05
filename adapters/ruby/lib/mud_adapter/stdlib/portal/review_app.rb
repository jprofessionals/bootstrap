# frozen_string_literal: true

begin
  require 'rugged'
rescue LoadError
  # rugged is optional; review portal features will be unavailable without it
end
require_relative 'base_app'

module MudAdapter
  module Stdlib
    module Portal
      class ReviewApp < BaseApp
        # rubocop:disable Metrics/BlockLength
        route do |r|
          require_builder!

          r.on String, String do |namespace, area_name|
            r.get '' do
              # TODO: Send DriverRequest to list MRs via MOP
              mrs = mop_client&.send_driver_request('mr_list_all', {
                namespace: namespace, name: area_name
              }) || []
              render_view('review_list', namespace: namespace, area_name: area_name,
                                         merge_requests: mrs, page_title: 'Merge Requests')
            end

            r.get Integer do |id|
              # TODO: Send DriverRequest to get MR via MOP
              mr = mop_client&.send_driver_request('mr_get', { id: id })
              halt_status(404) unless mr

              diff = generate_diff(namespace, area_name)
              render_view('review_detail', mr: mr, diff: diff,
                                           namespace: namespace, area_name: area_name,
                                           page_title: "MR ##{id}")
            end

            r.post 'new' do
              title = r.params['title']
              description = r.params['description']
              # TODO: Send DriverRequest to create MR via MOP
              mop_client&.send_driver_request('mr_create', {
                namespace: namespace, name: area_name, author: current_account,
                title: title, description: description
              })
              r.redirect "/review/#{namespace}/#{area_name}/"
            end

            r.post Integer, 'approve' do |id|
              # TODO: Send DriverRequest to approve MR via MOP
              mop_client&.send_driver_request('mr_approve', {
                id: id, username: current_account, comment: r.params['comment']
              })
              r.redirect "/review/#{namespace}/#{area_name}/#{id}"
            end

            r.post Integer, 'reject' do |id|
              # TODO: Send DriverRequest to reject MR via MOP
              mop_client&.send_driver_request('mr_reject', {
                id: id, username: current_account, reason: r.params['reason']
              })
              r.redirect "/review/#{namespace}/#{area_name}/#{id}"
            end

            r.post Integer, 'merge' do |id|
              # TODO: Send DriverRequest to merge MR via MOP
              mop_client&.send_driver_request('mr_merge', { id: id })
              r.redirect "/review/#{namespace}/#{area_name}/"
            end

            r.post Integer, 'close' do |id|
              # TODO: Send DriverRequest to close MR via MOP
              mop_client&.send_driver_request('mr_close', { id: id })
              r.redirect "/review/#{namespace}/#{area_name}/"
            end
          end
        end
        # rubocop:enable Metrics/BlockLength

        private

        def generate_diff(namespace, area_name)
          # TODO: Send DriverRequest to get repo_path via MOP
          # For now, try to find repo in conventional location
          repo_path = File.join('repos', namespace, "#{area_name}.git")
          return 'Repository not found' unless Dir.exist?(repo_path)

          repo = Rugged::Repository.new(repo_path)

          main = repo.references['refs/heads/main']&.target
          develop = repo.references['refs/heads/develop']&.target
          return '' unless main && develop

          diff = main.diff(develop)
          diff.patch
        rescue StandardError
          'Unable to generate diff'
        end
      end
    end
  end
end
