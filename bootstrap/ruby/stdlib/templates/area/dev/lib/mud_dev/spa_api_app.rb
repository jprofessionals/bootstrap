require 'roda'

module MudDev
  class SpaApiApp < Roda
    plugin :json
    plugin :all_verbs

    def self.build(routes_block:, area:, session_secret:)
      app = Class.new(self)
      app.plugin :sessions, secret: session_secret, cookie_options: { http_only: true, same_site: :lax }

      app.route do |r|
        response['access-control-allow-origin'] = '*'

        r.options do
          response['access-control-allow-methods'] = 'GET, POST, PUT, DELETE, PATCH, OPTIONS'
          response['access-control-allow-headers'] = 'Content-Type, Authorization'
          response.status = 204
          ''
        end

        result = routes_block.call(r, area, session)
        next result if result

        response.status = 404
        { error: 'not found' }
      end

      app
    end
  end
end
