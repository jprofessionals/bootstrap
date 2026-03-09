package mud.launcher

import kotlin.test.Test
import kotlin.test.assertEquals

class MopRouterTest {
    @Test
    fun `extracts area key from load_area message`() {
        val msg = mapOf(
            "type" to "load_area",
            "area_id" to mapOf("namespace" to "alice", "name" to "tavern"),
            "path" to "/data/world/alice/tavern",
            "db_url" to null,
        )
        assertEquals("alice/tavern", MopRouter.extractAreaKey(msg))
    }

    @Test
    fun `extracts area key from session_input via session registry`() {
        val router = MopRouter()
        router.registerSession(42L, "alice/tavern")
        assertEquals("alice/tavern", router.sessionAreaKey(42L))
    }

    @Test
    fun `returns null for unknown session`() {
        val router = MopRouter()
        assertEquals(null, router.sessionAreaKey(999L))
    }
}
