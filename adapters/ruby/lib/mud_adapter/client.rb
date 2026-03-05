# frozen_string_literal: true

require "socket"
require "msgpack"

module MudAdapter
  # MOP protocol client that communicates with the Rust driver over a Unix
  # domain socket using length-prefixed MessagePack frames.
  #
  # Wire format: [4 bytes big-endian u32 length][N bytes MessagePack payload]
  #
  # Thread-safe for writes via Mutex. Reads are expected to happen from a
  # single thread (the main message loop).
  class Client
    MAX_MESSAGE_SIZE = 16 * 1024 * 1024 # 16 MB, matching the Rust driver
    REQUEST_TIMEOUT = 10 # seconds

    class ProtocolError < StandardError; end
    class ConnectionClosed < StandardError; end
    class MessageTooLarge < ProtocolError; end
    class RequestTimeout < StandardError; end
    class DriverError < StandardError; end

    attr_reader :socket_path

    def initialize(socket_path)
      @socket_path = socket_path
      @socket = nil
      @write_mutex = Mutex.new
      @pending_requests = {}
      @pending_mutex = Mutex.new
      @request_counter = 0
      @counter_mutex = Mutex.new
    end

    # Connect to the MOP Unix socket and send the handshake message.
    def connect!
      @socket = UNIXSocket.new(@socket_path)
      send_handshake
    end

    # Close the connection.
    def close
      @socket&.close
      @socket = nil
    end

    # Send a message (Hash) as a length-prefixed MessagePack frame.
    # Thread-safe via Mutex.
    def send_message(hash)
      payload = MessagePack.pack(hash)

      if payload.bytesize > MAX_MESSAGE_SIZE
        raise MessageTooLarge, "Message size #{payload.bytesize} exceeds maximum #{MAX_MESSAGE_SIZE}"
      end

      @write_mutex.synchronize do
        @socket.write([payload.bytesize].pack("N"))
        @socket.write(payload)
        @socket.flush
      end
    end

    # Read the next message from the socket. Returns a Hash.
    # Blocks until a complete frame is available.
    # Raises ConnectionClosed on EOF.
    def read_message
      len_buf = read_exact(4)
      raise ConnectionClosed, "Connection closed by peer" if len_buf.nil?

      length = len_buf.unpack1("N")

      if length > MAX_MESSAGE_SIZE
        raise MessageTooLarge, "Incoming message size #{length} exceeds maximum #{MAX_MESSAGE_SIZE}"
      end

      payload = read_exact(length)
      raise ConnectionClosed, "Connection closed during message read" if payload.nil?

      MessagePack.unpack(payload)
    end

    # Check whether the client is connected.
    def connected?
      !@socket.nil? && !@socket.closed?
    end

    # Send a driver request and block until the response arrives.
    # This is called from web server threads while the main message loop
    # dispatches the response back via {#dispatch_response}.
    #
    # @param action [String] the driver request action name
    # @param params [Hash] request parameters
    # @return [Object] the result payload from the driver
    # @raise [RequestTimeout] if the driver does not respond in time
    # @raise [DriverError] if the driver returns an error
    def send_driver_request(action, params = {})
      request_id = next_request_id

      queue = Queue.new
      @pending_mutex.synchronize { @pending_requests[request_id] = queue }

      send_message(
        "type" => "driver_request",
        "request_id" => request_id,
        "action" => action,
        "params" => params
      )

      # Block until the main message loop pushes the response into our queue.
      response = nil
      begin
        response = queue.pop(timeout: REQUEST_TIMEOUT)
      rescue ThreadError
        # Ruby < 3.2 raises ThreadError when Queue#pop times out
      end

      @pending_mutex.synchronize { @pending_requests.delete(request_id) }

      if response.nil?
        raise RequestTimeout, "MOP request timed out after #{REQUEST_TIMEOUT}s: #{action}"
      end

      if response["type"] == "request_error"
        raise DriverError, "Driver error for '#{action}': #{response["error"]}"
      end

      response["result"]
    end

    # Dispatch a response message from the driver to the waiting request thread.
    # Called from the main message loop when a request_response or request_error
    # message is received.
    #
    # @param msg [Hash] the response message
    # @return [Boolean] true if a pending request was found and notified
    def dispatch_response(msg)
      request_id = msg["request_id"]
      queue = @pending_mutex.synchronize { @pending_requests[request_id] }

      if queue
        queue.push(msg)
        true
      else
        false
      end
    end

    private

    # Generate a monotonically increasing request ID.
    def next_request_id
      @counter_mutex.synchronize do
        @request_counter += 1
        @request_counter
      end
    end

    # Send the initial handshake identifying this adapter.
    def send_handshake
      send_message(
        "type" => "handshake",
        "adapter_name" => "mud-adapter-ruby",
        "language" => "ruby",
        "version" => MudAdapter::VERSION
      )
    end

    # Read exactly `n` bytes from the socket. Returns nil on EOF.
    def read_exact(n)
      buf = +""
      while buf.bytesize < n
        chunk = @socket.read(n - buf.bytesize)
        return nil if chunk.nil? || chunk.empty?

        buf << chunk
      end
      buf
    end
  end
end
