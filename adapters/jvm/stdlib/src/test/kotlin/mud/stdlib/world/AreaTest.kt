package mud.stdlib.world

import mud.stdlib.annotations.*
import kotlin.test.Test
import kotlin.test.assertEquals

@MudRoom
class TestRoom : Room() {
    override val name = "Test Room"
    override val description = "A test room."
}

@MudArea(webMode = WebMode.TEMPLATE)
class TestArea : Area() {
    @WebData
    fun data(): Map<String, Any> = mapOf(
        "room_count" to rooms.size,
        "area_name" to name
    )
}

class AreaTest {
    @Test
    fun `area registers rooms`() {
        val area = TestArea()
        area.name = "village"
        area.registerRoom("entrance", TestRoom())
        assertEquals(1, area.rooms.size)
        assertEquals("Test Room", area.rooms["entrance"]?.name)
    }

    @Test
    fun `WebData annotation is discoverable`() {
        val area = TestArea()
        area.name = "village"
        val method = area::class.java.methods.find {
            it.isAnnotationPresent(WebData::class.java)
        }
        assertEquals("data", method?.name)
        @Suppress("UNCHECKED_CAST")
        val result = method?.invoke(area) as Map<String, Any>
        assertEquals("village", result["area_name"])
    }
}
