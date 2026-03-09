package mud.mop.logging

import ch.qos.logback.classic.spi.ILoggingEvent
import ch.qos.logback.core.AppenderBase
import ch.qos.logback.classic.Level

class MopLogAppender(
    private val areaKey: String? = null,
    private val sender: (Map<String, Any?>) -> Unit,
) : AppenderBase<ILoggingEvent>() {

    override fun append(event: ILoggingEvent) {
        val level = when (event.level) {
            Level.ERROR -> "error"
            Level.WARN -> "warn"
            Level.INFO -> "info"
            Level.DEBUG -> "debug"
            Level.TRACE -> "trace"
            else -> "info"
        }
        log(level, event.formattedMessage)
    }

    fun log(level: String, message: String) {
        sender(mapOf(
            "type" to "log",
            "level" to level,
            "message" to message,
            "area" to areaKey,
        ))
    }
}
