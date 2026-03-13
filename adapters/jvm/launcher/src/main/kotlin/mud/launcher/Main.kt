package mud.launcher

import mud.mop.client.MopClient
import org.slf4j.LoggerFactory
import kotlinx.coroutines.*

private val logger = LoggerFactory.getLogger("mud.launcher.Main")

fun main(args: Array<String>) {
    val socketPath = parseSocketPath(args)

    logger.info("JVM Adapter Launcher starting, connecting to {}", socketPath)

    val client = MopClient.connect(
        socketPath = socketPath,
        adapterName = "mud-adapter-jvm",
        language = "kotlin",
        version = "0.1.0",
    )

    val router = MopRouter()
    val processManager = AreaProcessManager(
        onSendToDriver = { msg ->
            if (msg["type"] == "register_area_web") {
                // Convert to driver_request so the driver handles it as a request
                val requestId = System.nanoTime()
                client.sendMessage(mapOf(
                    "type" to "driver_request",
                    "request_id" to requestId,
                    "action" to "register_area_web",
                    "params" to mapOf(
                        "area_key" to msg["area_key"],
                        "socket_path" to msg["socket_path"],
                    ),
                ))
            } else {
                client.sendMessage(msg)
            }
        },
        onLog = { level, message ->
            client.sendMessage(mapOf(
                "type" to "log",
                "level" to level,
                "message" to message,
                "area" to null,
            ))
        },
    )

    // Send handshake
    client.sendHandshake()
    logger.info("Handshake sent")

    client.onMessage = { msg -> dispatchDriverMessage(msg, router, processManager, client) }

    // Start read loop on background thread
    val readThread = Thread({ client.readLoop() }, "mop-read-loop")
    readThread.isDaemon = true
    readThread.start()

    // Wait for read loop to finish (connection closed)
    readThread.join()

    processManager.shutdown()
    logger.info("Launcher shut down")
}

private fun dispatchDriverMessage(
    msg: Map<String, Any?>,
    router: MopRouter,
    processManager: AreaProcessManager,
    client: MopClient,
) {
    when (msg["type"]) {
        "load_area", "reload_area" -> {
            val areaKey = MopRouter.extractAreaKey(msg) ?: return
            @Suppress("UNCHECKED_CAST")
            val areaId = msg["area_id"] as Map<String, String>
            val path = msg["path"] as? String ?: return
            val dbUrl = msg["db_url"] as? String
            processManager.loadArea(areaId, path, dbUrl)
        }

        "unload_area" -> {
            val areaKey = MopRouter.extractAreaKey(msg) ?: return
            processManager.requestUnload(areaKey)
        }

        "session_start" -> {
            val sessionId = (msg["session_id"] as Number).toLong()
            // Register session with the first running area (single-area default).
            // In multi-area setups, the driver would need to include area routing info.
            val areaKey = processManager.firstRunningAreaKey()
            if (areaKey != null) {
                router.registerSession(sessionId, areaKey)
                processManager.routeToArea(areaKey, msg)
            } else {
                logger.warn("No running area to assign session {} to", sessionId)
            }
        }

        "session_input" -> {
            val sessionId = (msg["session_id"] as Number).toLong()
            val areaKey = router.sessionAreaKey(sessionId) ?: return
            processManager.routeToArea(areaKey, msg)
        }

        "session_end" -> {
            val sessionId = (msg["session_id"] as Number).toLong()
            val areaKey = router.sessionAreaKey(sessionId) ?: return
            processManager.routeToArea(areaKey, msg)
            router.unregisterSession(sessionId)
        }

        "ping" -> {
            client.sendMessage(mapOf("type" to "pong", "seq" to msg["seq"]))
        }

        "configure" -> {
            logger.info("Received configure message")
            // Store stdlib DB URL if needed
        }

        "get_web_data", "check_builder_access" -> {
            // Route to appropriate area
            val areaKey = msg["area_key"] as? String
            if (areaKey != null) {
                processManager.routeToArea(areaKey, msg)
            }
        }

        else -> {
            logger.warn("Unhandled driver message type: {}", msg["type"])
        }
    }
}

private fun parseSocketPath(args: Array<String>): String {
    val idx = args.indexOf("--socket")
    if (idx >= 0 && idx + 1 < args.size) return args[idx + 1]
    throw IllegalArgumentException("Usage: launcher --socket <path>")
}
