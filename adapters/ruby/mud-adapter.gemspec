# frozen_string_literal: true

Gem::Specification.new do |s|
  s.name        = "mud-adapter"
  s.version     = "0.1.0"
  s.summary     = "MUD driver Ruby adapter"
  s.description = "Ruby language adapter for the MUD driver, communicating via the MOP protocol over Unix sockets."
  s.authors     = ["MUD Team"]
  s.license     = "MIT"

  s.required_ruby_version = ">= 3.1"

  s.files         = Dir["lib/**/*.rb", "bin/*"]
  s.executables   = ["mud-adapter"]
  s.require_paths = ["lib"]

  s.add_dependency "msgpack", "~> 1.7"
  s.add_dependency "roda", "~> 3.0"
  s.add_dependency "rack", "~> 3.0"
  s.add_dependency "puma", "~> 6.0"
  s.add_dependency "sequel", "~> 5.0"
  s.add_dependency "pg", "~> 1.5"
end
