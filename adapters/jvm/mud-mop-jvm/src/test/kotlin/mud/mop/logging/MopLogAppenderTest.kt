package mud.mop.logging

import kotlin.test.Test
import kotlin.test.assertEquals

class MopLogAppenderTest {
    @Test
    fun `captures log events and produces MOP messages`() {
        val captured = mutableListOf<Map<String, Any?>>()
        val appender = MopLogAppender(areaKey = "test/village") { msg -> captured.add(msg) }

        appender.log("info", "area loaded successfully")
        appender.log("error", "something went wrong")

        assertEquals(2, captured.size)
        assertEquals("log", captured[0]["type"])
        assertEquals("info", captured[0]["level"])
        assertEquals("area loaded successfully", captured[0]["message"])
        assertEquals("test/village", captured[0]["area"])
        assertEquals("error", captured[1]["level"])
    }
}
