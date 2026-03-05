# Uncomment the next line to switch to SPA mode:
# web_mode :spa

web_data do |area, helpers|
  {
    area_name: File.basename(area.path),
    room_count: area.rooms.size,
    item_count: area.items.size,
    npc_count: area.npcs.size,
    server_name: helpers.server_name,
    players_online: helpers.total_players_online
  }
end

# Simple API routes (legacy) — available in both ERB and SPA modes:
# web_routes do |r, area, _session|
#   r.get 'status' do
#     { status: 'ok', area: File.basename(area.path) }
#   end
# end

# Full Rack app — use for richer backends. Return 404 to fall through to
# SPA/ERB frontend serving. Receives work_path so you can require area code.
# web_app do |work_path|
#   ->(env) {
#     req = Rack::Request.new(env)
#     case [req.request_method, req.path_info]
#     when ['GET', '/api/status']
#       [200, { 'content-type' => 'application/json' }, ['{"status":"ok"}'.freeze]]
#     else
#       [404, {}, []]
#     end
#   }
# end
