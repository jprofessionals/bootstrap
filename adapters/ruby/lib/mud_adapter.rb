# frozen_string_literal: true

require_relative "mud_adapter/client"
require_relative "mud_adapter/session_handler"
require_relative "mud_adapter/area_loader"
require_relative "mud_adapter/stdlib_runtime"
require_relative "mud_adapter/web_server"

module MudAdapter
  VERSION = "0.1.0"
end

MudAdapter::StdlibRuntime.load!

# Alias MUD -> MudAdapter so that area code written for the Ruby MUD driver
# (e.g. `MUD::Stdlib::World::Room` in mud_aliases.rb) works unchanged.
MUD = MudAdapter unless defined?(MUD)
