import mud.stdlib.annotations.MudRoom
import mud.stdlib.world.Room

@MudRoom
class Entrance : Room() {
    override val name = "The Entrance"
    override val description = """
        You stand at the entrance of {{area_name}}.
        Stone walls rise around you, cool and damp to the touch.
        A passage leads north into the darkness.
    """.trimIndent()

    init {
        // exit("north", "rooms.hall")
    }
}
