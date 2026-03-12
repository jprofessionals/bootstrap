# frozen_string_literal: true

module MudAdapter
  # Manages player sessions and dispatches commands using the stdlib.
  #
  # Each session is identified by a numeric session_id (u64 on the Rust side).
  # The handler uses stdlib Area/Room objects for the world and the stdlib
  # command Parser for input processing.
  class SessionHandler
    DIRECTION_VERBS = %w[north south east west n s e w
                         northeast northwest southeast southwest
                         ne nw se sw up down].freeze

    DIRECTION_ALIASES = {
      "n" => "north", "s" => "south", "e" => "east", "w" => "west",
      "ne" => "northeast", "nw" => "northwest",
      "se" => "southeast", "sw" => "southwest"
    }.freeze

    def initialize(client, area_loader)
      @client = client
      @area_loader = area_loader
      @sessions = {} # session_id => { username:, room_key:, area_key: }
    end

    # Start a new player session. Sends welcome text and initial room
    # description to the player.
    def start_session(session_id, username)
      start_room = find_start_room

      @sessions[session_id] = {
        username: username,
        room_key: start_room[:key],
        area_key: start_room[:area_key]
      }

      send_output(session_id, welcome_text(username))
      send_output(session_id, room_description(session_id))
      send_output(session_id, "\n> ")
    end

    # Handle a line of input from a player session.
    def handle_input(session_id, line)
      session = @sessions[session_id]
      unless session
        send_output(session_id, "Error: session not found.\n> ")
        return
      end

      command = MudAdapter::Stdlib::Commands::Parser.parse(line.strip)
      if command.nil?
        send_output(session_id, "> ")
        return
      end

      result = if command.builder_command?
                 handle_builder_command(session_id, command)
               else
                 handle_game_command(session_id, command)
               end

      send_output(session_id, "#{result}\n> ")
    end

    # End a player session and clean up state.
    def end_session(session_id)
      session = @sessions.delete(session_id)
      return unless session

      log_info("Session ended for #{session[:username]} (#{session_id})")
    end

    # Create a web session (no MOP output). Returns [session_id, initial_output].
    def create_web_session(username)
      session_id = next_web_session_id
      start_room = find_start_room

      @sessions[session_id] = {
        username: username,
        room_key: start_room[:key],
        area_key: start_room[:area_key]
      }

      output = welcome_text(username) + room_description(session_id)
      [session_id, output]
    end

    # Execute a command for a web session. Returns output string.
    def execute_command(session_id, line)
      session = @sessions[session_id]
      return "Error: session not found." unless session

      command = MudAdapter::Stdlib::Commands::Parser.parse(line.strip)
      return "" unless command

      if command.builder_command?
        handle_builder_command(session_id, command)
      else
        handle_game_command(session_id, command)
      end
    end

    # End a web session.
    def destroy_web_session(session_id)
      end_session(session_id)
    end

    def total_players_online
      @sessions.size
    end

    private

    # Find a starting room for a new player.
    # Uses the first room of the first loaded area, or a void fallback.
    def find_start_room
      room = @area_loader.first_room
      return room if room

      # No areas loaded yet -- return a void placeholder
      { key: "__void__", area_key: "__none__" }
    end

    # Dispatch a game command (non-builder).
    def handle_game_command(session_id, command)
      verb = command.verb.to_s

      case verb
      when "look", "l"
        room_description(session_id)
      when "help", "?"
        help_text
      when "who"
        who_list
      when "say"
        handle_say(session_id, command.args)
      when *DIRECTION_VERBS
        direction = DIRECTION_ALIASES[verb] || verb
        move_player(session_id, direction)
      else
        "Unknown command: '#{verb}'. Type 'help' for a list of commands."
      end
    end

    # Dispatch a builder command (starts with @).
    def handle_builder_command(session_id, command)
      session = @sessions[session_id]
      builder = MudAdapter::Stdlib::Commands::Builder.new(
        client: @client,
        username: session[:username]
      )
      builder.handle(command)
    end

    # Move a player in the given direction, if an exit exists.
    def move_player(session_id, direction)
      session = @sessions[session_id]
      room = @area_loader.find_room(session[:area_key], session[:room_key])
      return "You can't go that way." unless room

      exits = room.exits || {}
      # Check both string and symbol keys for the direction
      target = exits[direction.to_s] || exits[direction.to_sym]
      unless target
        return "You can't go #{direction} from here."
      end

      session[:room_key] = target.to_s

      # Check if the target room is in a different area
      unless @area_loader.find_room(session[:area_key], target.to_s)
        new_area = @area_loader.area_for_room(target.to_s)
        session[:area_key] = new_area if new_area
      end

      "You move #{direction}.\n\n#{room_description(session_id)}"
    end

    # Build the room description string for the player's current room.
    def room_description(session_id)
      session = @sessions[session_id]
      room = @area_loader.find_room(session[:area_key], session[:room_key])

      unless room
        return "You are in an empty void. There is nothing here."
      end

      lines = []
      lines << (room.title || room.class.name.split("::").last)
      lines << (room.description || "")

      exits = room.exits
      if exits && !exits.empty?
        lines << "Exits: #{exits.keys.join(', ')}"
      else
        lines << "There are no obvious exits."
      end

      lines.join("\n")
    end

    # Handle the say command.
    def handle_say(session_id, args)
      session = @sessions[session_id]
      message = args.join(" ")
      return "Say what?" if message.empty?

      "#{session[:username]} says: #{message}"
    end

    # Send text output to a player session via the MOP client.
    def send_output(session_id, text)
      @client.send_message(
        "type" => "session_output",
        "session_id" => session_id,
        "text" => text
      )
    end

    # Log an info message via the MOP client.
    def log_info(message)
      @client.send_message(
        "type" => "log",
        "level" => "info",
        "message" => message,
        "area" => nil
      )
    end

    def welcome_text(username)
      <<~TEXT
        ====================================
          Welcome, #{username}!
          Type 'help' for available commands.
        ====================================

      TEXT
    end

    def help_text
      <<~TEXT.chomp
        Available commands:
          look (l)           - Look around the current room
          north/south/e/w    - Move in a direction
          say <message>      - Say something
          who                - See who is online
          help (?)           - Show this help message
        Builder commands:
          @repo              - Manage git repositories
          @project           - Manage projects
          @review            - Merge request workflow
      TEXT
    end

    def who_list
      if @sessions.empty?
        "No one is online."
      else
        names = @sessions.values.map { |s| "  #{s[:username]}" }
        "Players online:\n#{names.join("\n")}"
      end
    end

    # Generate negative session IDs for web sessions to avoid collision
    # with SSH positive IDs.
    def next_web_session_id
      @next_web_session_id ||= 0
      @next_web_session_id -= 1
    end
  end
end
