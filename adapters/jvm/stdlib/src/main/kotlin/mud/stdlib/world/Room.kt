package mud.stdlib.world

open class Room : GameObject() {
    private val exits = mutableMapOf<String, String>()

    fun exit(direction: String, to: String) {
        exits[direction] = to
    }

    fun exits(): Map<String, String> = exits
    fun hasExit(direction: String): Boolean = direction in exits

    open fun onEnter(player: String) {}
}
