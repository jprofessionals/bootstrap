import mud.stdlib.annotations.*
import mud.stdlib.world.Area

@mud.stdlib.annotations.MudArea(webMode = WebMode.TEMPLATE)
class MudArea : Area() {

    @WebData
    fun templateData(): Map<String, Any> = mapOf(
        "area_name" to name,
        "namespace" to namespace,
        "room_count" to rooms.size,
        "item_count" to items.size,
        "npc_count" to npcs.size,
    )
}
