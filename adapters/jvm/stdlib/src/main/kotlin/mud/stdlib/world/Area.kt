package mud.stdlib.world

open class Area {
    var name: String = ""
    var namespace: String = ""
    var path: String = ""

    val rooms = mutableMapOf<String, Room>()
    val items = mutableMapOf<String, Item>()
    val npcs = mutableMapOf<String, NPC>()
    val daemons = mutableMapOf<String, Daemon>()

    fun registerRoom(key: String, room: Room) { rooms[key] = room }
    fun registerItem(key: String, item: Item) { items[key] = item }
    fun registerNPC(key: String, npc: NPC) { npcs[key] = npc }
    fun registerDaemon(key: String, daemon: Daemon) { daemons[key] = daemon }
}
