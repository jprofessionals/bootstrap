package mud.stdlib.world

open class Item : GameObject() {
    open val portable: Boolean = false

    open fun onUse(player: String, target: String?) {}
}
