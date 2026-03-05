# frozen_string_literal: true

require 'json'
require 'fileutils'

module MudAdapter
  class AreaLogger
    MAX_RING_SIZE = 200

    # @param client [MudAdapter::Client] MOP client for sending log messages to driver
    def initialize(client)
      @client = client
      @area_paths = {}  # area_key => work_path
      @rings = {}       # area_key => Array of log entry hashes
      @mutex = Mutex.new
    end

    # Register an area's work path (called during load_area)
    def register_area(area_key, work_path)
      @mutex.synchronize do
        @area_paths[area_key] = work_path
        # Seed in-memory ring from existing log file
        @rings[area_key] ||= load_from_file(work_path)
      end
    end

    # Unregister an area (called during unload_area)
    def unregister_area(area_key)
      @mutex.synchronize do
        @area_paths.delete(area_key)
        @rings.delete(area_key)
      end
    end

    # Log an event for an area
    def log(area_key, level, event, message, backtrace: nil)
      entry = {
        'ts' => Time.now.utc.strftime('%Y-%m-%dT%H:%M:%SZ'),
        'level' => level.to_s,
        'area' => area_key,
        'event' => event.to_s,
        'message' => message.to_s
      }
      entry['backtrace'] = backtrace.to_s if backtrace

      @mutex.synchronize do
        # Append to in-memory ring
        ring = (@rings[area_key] ||= [])
        ring << entry
        ring.shift while ring.size > MAX_RING_SIZE

        # Append to per-area log file
        if (work_path = @area_paths[area_key])
          write_to_file(work_path, entry)
        end
      end

      # Send to driver for master log (outside mutex to avoid blocking)
      send_to_driver(area_key, level, event, message, backtrace)
    end

    # Get recent log entries for an area
    def recent(area_key, limit: 50, level: nil)
      @mutex.synchronize do
        ring = @rings[area_key] || []
        entries = if level && level != 'all'
                    ring.select { |e| severity(e['level']) >= severity(level) }
                  else
                    ring
                  end
        entries.last([limit, MAX_RING_SIZE].min)
      end
    end

    private

    def severity(level)
      case level.to_s
      when 'error' then 3
      when 'warn' then 2
      when 'info' then 1
      else 0
      end
    end

    def write_to_file(work_path, entry)
      mud_dir = File.join(work_path, '.mud')
      FileUtils.mkdir_p(mud_dir) unless Dir.exist?(mud_dir)
      log_path = File.join(mud_dir, 'reload.log')
      File.open(log_path, 'a') { |f| f.puts(entry.to_json) }
    rescue StandardError => e
      $stderr.puts "[area-logger] Failed to write log file: #{e.message}"
    end

    def load_from_file(work_path)
      log_path = File.join(work_path, '.mud', 'reload.log')
      return [] unless File.exist?(log_path)

      entries = []
      File.foreach(log_path) do |line|
        entry = JSON.parse(line.strip) rescue next
        entries << entry
      end
      # Keep only the most recent entries
      entries.last(MAX_RING_SIZE)
    rescue StandardError
      []
    end

    def send_to_driver(area_key, level, event, message, backtrace)
      log_message = backtrace ? "[#{event}] #{message}\n#{backtrace}" : "[#{event}] #{message}"
      @client.send_message(
        'type' => 'log',
        'level' => level.to_s,
        'message' => log_message,
        'area' => area_key
      )
    rescue StandardError
      # Best-effort: don't crash if MOP send fails
      nil
    end
  end
end
