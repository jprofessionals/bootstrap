# frozen_string_literal: true

module MudAdapter
  module Stdlib
    module Portal
      class App < BaseApp
        route do |r|
          r.on 'account' do
            r.run AccountApp
          end

          r.on 'play' do
            redirect_unless_trailing_slash(r)
            r.run PlayApp
          end

          r.on 'editor' do
            redirect_unless_trailing_slash(r)
            r.run EditorApp
          end

          r.on 'git' do
            redirect_unless_trailing_slash(r)
            r.run GitApp
          end

          r.on 'review' do
            redirect_unless_trailing_slash(r)
            r.run ReviewApp
          end

          r.on 'builder' do
            redirect_unless_trailing_slash(r)
            r.run BuilderApp
          end

          r.on 'project' do
            redirect_unless_trailing_slash(r)
            r.run BuilderApp
          end

          r.root do
            render_view(:welcome, server_name: server_name, page_title: server_name,
                                  account: session['account'])
          end
        end

        private

        def redirect_unless_trailing_slash(request)
          return unless request.remaining_path.empty?

          request.redirect "#{request.matched_path}/"
        end
      end
    end
  end
end
