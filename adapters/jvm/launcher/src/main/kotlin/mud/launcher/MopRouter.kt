package mud.launcher

import org.slf4j.LoggerFactory
import java.util.concurrent.ConcurrentHashMap

class MopRouter {
    private val logger = LoggerFactory.getLogger(MopRouter::class.java)
    private val sessionToArea = ConcurrentHashMap<Long, String>()

    companion object {
        fun extractAreaKey(msg: Map<String, Any?>): String? {
            @Suppress("UNCHECKED_CAST")
            val areaId = msg["area_id"] as? Map<String, Any?> ?: return null
            val ns = areaId["namespace"] as? String ?: return null
            val name = areaId["name"] as? String ?: return null
            return "$ns/$name"
        }
    }

    fun registerSession(sessionId: Long, areaKey: String) {
        sessionToArea[sessionId] = areaKey
    }

    fun unregisterSession(sessionId: Long) {
        sessionToArea.remove(sessionId)
    }

    fun sessionAreaKey(sessionId: Long): String? = sessionToArea[sessionId]
}
