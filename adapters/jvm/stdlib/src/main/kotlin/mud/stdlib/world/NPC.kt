package mud.stdlib.world

open class NPC : GameObject() {
    open val location: String? = null

    open fun onTalk(player: String) {}
}
