# frozen_string_literal: true

require_relative "mud_adapter/client"
require_relative "mud_adapter/session_handler"
require_relative "mud_adapter/area_loader"
require_relative "mud_adapter/stdlib"
require_relative "mud_adapter/stdlib/web/rack_app"
require_relative "mud_adapter/stdlib/portal/base_app"
require_relative "mud_adapter/stdlib/portal/account_app"
require_relative "mud_adapter/stdlib/portal/play_app"
require_relative "mud_adapter/stdlib/portal/editor_app"
require_relative "mud_adapter/stdlib/portal/git_app"
require_relative "mud_adapter/stdlib/portal/review_app"
require_relative "mud_adapter/stdlib/portal/builder_app"
require_relative "mud_adapter/stdlib/portal/app"
require_relative "mud_adapter/web_server"

module MudAdapter
  VERSION = "0.1.0"
end

# Alias MUD -> MudAdapter so that area code written for the Ruby MUD driver
# (e.g. `MUD::Stdlib::World::Room` in mud_aliases.rb) works unchanged.
MUD = MudAdapter unless defined?(MUD)
