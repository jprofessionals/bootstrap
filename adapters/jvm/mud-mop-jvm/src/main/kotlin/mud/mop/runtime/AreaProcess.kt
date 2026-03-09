package mud.mop.runtime

import mud.mop.codec.MopCodec
import mud.mop.migrations.FlywayRunner
import org.slf4j.LoggerFactory
import java.io.File
import java.io.InputStream
import java.io.OutputStream

class AreaProcess(
    private val input: InputStream,
    private val output: OutputStream,
    private val areaId: Map<String, String>,
    private val areaPath: String,
    private val dbUrl: String?,
    private val scanPackage: String,
) {
    private val logger = LoggerFactory.getLogger(AreaProcess::class.java)
    private val writeLock = Any()
    private lateinit var runtime: AreaRuntime
    private val areaKey = "${areaId["namespace"]}/${areaId["name"]}"
    private var webServer: KtorWebServer? = null

    fun run() {
        logger.info("Starting area process for {}", areaKey)

        // Run migrations if db_url provided
        if (dbUrl != null && FlywayRunner.hasMigrations(areaPath)) {
            try {
                FlywayRunner.run(areaPath, dbUrl)
            } catch (e: Exception) {
                logger.error("Migration failed for {}", areaKey, e)
                sendMessage(mapOf(
                    "type" to "area_error",
                    "area_id" to areaId,
                    "error" to "migration failed: ${e.message}",
                ))
                return
            }
        }

        // Scan classpath for game objects
        try {
            runtime = AreaRuntime(scanPackage)
            runtime.area.name = areaId["name"] ?: ""
            runtime.area.namespace = areaId["namespace"] ?: ""
            runtime.area.path = areaPath
        } catch (e: Exception) {
            logger.error("Failed to initialize area {}", areaKey, e)
            sendMessage(mapOf(
                "type" to "area_error",
                "area_id" to areaId,
                "error" to "init failed: ${e.message}",
            ))
            return
        }

        // Start Ktor web server if framework is available
        val framework = readFramework()
        if (framework != null && framework != "none" && KtorWebServer.isAvailable()) {
            try {
                val server = KtorWebServer(runtime, areaKey, dbUrl)
                server.start()
                webServer = server

                // Tell the driver to proxy API requests to us
                sendMessage(mapOf(
                    "type" to "register_area_web",
                    "area_key" to areaKey,
                    "socket_path" to "tcp:127.0.0.1:${server.port}",
                ))
            } catch (e: Exception) {
                logger.error("Failed to start web server for {}", areaKey, e)
                // Non-fatal — area still works for game sessions
            }
        }

        // Notify area loaded
        sendMessage(mapOf(
            "type" to "area_loaded",
            "area_id" to areaId,
        ))

        // Message loop
        try {
            while (true) {
                val msg = MopCodec.readFrame(input)
                dispatch(msg)
            }
        } catch (_: MopCodec.ConnectionClosed) {
            logger.info("Connection closed for area {}", areaKey)
        } catch (e: Exception) {
            logger.error("Error in area {} message loop", areaKey, e)
        } finally {
            webServer?.stop()
        }
    }

    private fun dispatch(msg: Map<String, Any?>) {
        when (msg["type"]) {
            "ping" -> sendMessage(mapOf("type" to "pong", "seq" to msg["seq"]))

            "session_start" -> {
                logger.info("Session started: {}", msg["session_id"])
            }

            "session_input" -> {
                val sessionId = msg["session_id"]
                val line = msg["line"] as? String ?: ""
                handleInput(sessionId, line)
            }

            "session_end" -> {
                logger.info("Session ended: {}", msg["session_id"])
            }

            "get_web_data" -> {
                val requestId = msg["request_id"]
                val data = runtime.getWebData() ?: emptyMap()
                sendMessage(mapOf(
                    "type" to "call_result",
                    "request_id" to requestId,
                    "result" to data,
                ))
            }

            "check_builder_access" -> {
                val requestId = msg["request_id"]
                sendMessage(mapOf(
                    "type" to "call_result",
                    "request_id" to requestId,
                    "result" to mapOf("allowed" to true),
                ))
            }

            else -> logger.warn("Unhandled message type: {}", msg["type"])
        }
    }

    private fun handleInput(sessionId: Any?, line: String) {
        // Basic command handling -- areas can override
        sendMessage(mapOf(
            "type" to "session_output",
            "session_id" to sessionId,
            "text" to "You said: $line\n",
        ))
    }

    private fun sendMessage(msg: Map<String, Any?>) {
        synchronized(writeLock) {
            MopCodec.writeFrame(output, msg)
        }
    }

    /**
     * Read the framework field from mud.yaml in the area path.
     */
    private fun readFramework(): String? {
        val yamlFile = File(areaPath, "mud.yaml")
        if (!yamlFile.exists()) return null
        // Simple YAML parsing — just extract the framework line
        yamlFile.readLines().forEach { line ->
            val trimmed = line.trim()
            if (trimmed.startsWith("framework:")) {
                return trimmed.substringAfter("framework:").trim()
            }
        }
        return null
    }

    companion object {
        /**
         * Entry point for child JVM processes spawned by the launcher.
         * Expected env vars: MUD_SOCKET_PATH, MUD_AREA_NS, MUD_AREA_NAME,
         * MUD_AREA_PATH, MUD_DB_URL (optional), MUD_SCAN_PACKAGE
         */
        @JvmStatic
        fun main(args: Array<String>) {
            val socketPath = System.getenv("MUD_SOCKET_PATH")
                ?: throw IllegalArgumentException("MUD_SOCKET_PATH not set")
            val ns = System.getenv("MUD_AREA_NS")
                ?: throw IllegalArgumentException("MUD_AREA_NS not set")
            val name = System.getenv("MUD_AREA_NAME")
                ?: throw IllegalArgumentException("MUD_AREA_NAME not set")
            val path = System.getenv("MUD_AREA_PATH")
                ?: throw IllegalArgumentException("MUD_AREA_PATH not set")
            val dbUrl = System.getenv("MUD_DB_URL")
            val scanPackage = System.getenv("MUD_SCAN_PACKAGE") ?: ""

            val socket = org.newsclub.net.unix.AFUNIXSocket.newInstance()
            socket.connect(org.newsclub.net.unix.AFUNIXSocketAddress.of(java.io.File(socketPath)))

            val process = AreaProcess(
                input = socket.getInputStream(),
                output = socket.getOutputStream(),
                areaId = mapOf("namespace" to ns, "name" to name),
                areaPath = path,
                dbUrl = dbUrl,
                scanPackage = scanPackage,
            )
            process.run()
        }
    }
}
