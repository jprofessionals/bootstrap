package mud.mop.runtime

import mud.stdlib.annotations.*
import mud.stdlib.world.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull

// Test classes -- these live in the test classpath so ClassGraph can find them.
@MudRoom
class ScanTestRoom : Room() {
    override val name = "Scanned Room"
    override val description = "Found by scanner."
}

@MudNPC
class ScanTestNPC : NPC() {
    override val name = "Scanned NPC"
}

@MudArea(webMode = WebMode.TEMPLATE)
class ScanTestArea : Area() {
    @WebData
    fun data(): Map<String, Any> = mapOf("room_count" to rooms.size)
}

class AreaRuntimeTest {
    @Test
    fun `scans and populates area from annotated classes`() {
        val runtime = AreaRuntime("mud.mop.runtime")
        val area = runtime.area
        assertNotNull(area)
        assertEquals(WebMode.TEMPLATE, runtime.webMode)
        // Room should be registered
        assertEquals(1, area.rooms.size)
        assertEquals("Scanned Room", area.rooms.values.first().name)
        // NPC should be registered
        assertEquals(1, area.npcs.size)
    }

    @Test
    fun `invokes WebData method`() {
        val runtime = AreaRuntime("mud.mop.runtime")
        runtime.area.registerRoom("extra", ScanTestRoom())
        val data = runtime.getWebData()
        assertNotNull(data)
        assertEquals(2, data["room_count"]) // original + extra
    }
}
