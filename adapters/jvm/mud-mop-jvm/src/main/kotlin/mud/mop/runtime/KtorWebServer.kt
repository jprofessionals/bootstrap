package mud.mop.runtime

import io.ktor.http.*
import io.ktor.server.application.*
import io.ktor.server.engine.*
import io.ktor.server.netty.*
import io.ktor.server.response.*
import io.ktor.server.routing.*
import org.slf4j.LoggerFactory
import java.net.ServerSocket

/**
 * Starts a Ktor embedded HTTP server for areas with a JVM framework.
 * This class uses Ktor (compileOnly — only available at runtime when the
 * area's fat JAR includes Ktor dependencies).
 *
 * The server listens on a random TCP port on 0.0.0.0 and serves area API routes.
 */
class KtorWebServer(
    private val runtime: AreaRuntime,
    private val areaKey: String,
    private val dbUrl: String?,
) {
    private val logger = LoggerFactory.getLogger(KtorWebServer::class.java)
    private var server: EmbeddedServer<*, *>? = null

    val port: Int = findFreePort()

    fun start() {
        logger.info("Starting Ktor web server for {} on port {}", areaKey, port)

        server = embeddedServer(Netty, port = port, host = "0.0.0.0") {
            routing {
                get("/api/status") {
                    val json = """{"status":"ok","area":"$areaKey","framework":"ktor"}"""
                    call.respondText(json, ContentType.Application.Json)
                }

                get("/api/web-data") {
                    val data = runtime.getWebData() ?: emptyMap()
                    // Simple JSON serialization for the web data map
                    val json = buildJsonString(data)
                    call.respondText(json, ContentType.Application.Json)
                }
            }
        }.start(wait = false)

        logger.info("Ktor web server started for {} on port {}", areaKey, port)
    }

    fun stop() {
        server?.stop(1000, 2000)
    }

    private fun buildJsonString(map: Map<String, Any?>): String {
        val entries = map.entries.joinToString(",") { (k, v) ->
            "\"$k\":${valueToJson(v)}"
        }
        return "{$entries}"
    }

    private fun valueToJson(value: Any?): String = when (value) {
        null -> "null"
        is String -> "\"${value.replace("\"", "\\\"")}\""
        is Number -> value.toString()
        is Boolean -> value.toString()
        is Map<*, *> -> {
            @Suppress("UNCHECKED_CAST")
            buildJsonString(value as Map<String, Any?>)
        }
        else -> "\"$value\""
    }

    companion object {
        fun isAvailable(): Boolean {
            return try {
                Class.forName("io.ktor.server.engine.EmbeddedServerKt")
                true
            } catch (_: ClassNotFoundException) {
                false
            }
        }

        private fun findFreePort(): Int {
            ServerSocket(0).use { return it.localPort }
        }
    }
}
