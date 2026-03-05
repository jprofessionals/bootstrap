# Minimal extraction from MUD::Stdlib::World::WebDataDSL
# Self-contained — no driver dependency
module MudDev
  class WebDataDSL
    attr_reader :data_block, :routes_block

    def initialize
      @data_block = nil
      @routes_block = nil
      @mode = :erb
    end

    def web_mode(mode)
      @mode = mode
    end

    def spa_mode?
      @mode == :spa
    end

    def web_data(&block)
      @data_block = block
    end

    def web_routes(&block)
      @routes_block = block
    end

    def web_app(&)
      # ignored in dev mode
    end

    def self.evaluate(path)
      return nil unless File.exist?(path)

      dsl = new
      content = File.read(path)
      dsl.instance_eval(content, path)
      dsl
    end
  end
end
