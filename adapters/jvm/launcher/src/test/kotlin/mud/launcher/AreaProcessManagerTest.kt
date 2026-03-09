package mud.launcher

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class AreaProcessManagerTest {
    @Test
    fun `tracks area states through lifecycle`() {
        val manager = AreaProcessManager(
            onSendToDriver = {},
            onLog = { _, _ -> },
        )

        val areaId = mapOf("namespace" to "test", "name" to "village")
        val areaKey = "test/village"

        assertFalse(manager.hasArea(areaKey))

        manager.markBuilding(areaKey, areaId)
        assertTrue(manager.hasArea(areaKey))
        assertEquals(AreaState.BUILDING, manager.getState(areaKey))

        manager.markRunning(areaKey)
        assertEquals(AreaState.RUNNING, manager.getState(areaKey))

        manager.remove(areaKey)
        assertFalse(manager.hasArea(areaKey))
    }

    @Test
    fun `unload during build marks as pending_unload`() {
        val manager = AreaProcessManager(
            onSendToDriver = {},
            onLog = { _, _ -> },
        )

        val areaKey = "test/village"
        manager.markBuilding(areaKey, mapOf("namespace" to "test", "name" to "village"))
        manager.requestUnload(areaKey)
        assertEquals(AreaState.PENDING_UNLOAD, manager.getState(areaKey))
    }
}
