package mud.mop.runtime

import io.github.classgraph.ClassGraph
import mud.stdlib.annotations.*
import mud.stdlib.world.*
import org.slf4j.LoggerFactory
import java.lang.reflect.Method

class AreaRuntime(scanPackage: String) {
    private val logger = LoggerFactory.getLogger(AreaRuntime::class.java)

    val area: Area
    val webMode: WebMode
    private val webDataMethod: Method?
    private val webDataTarget: Any?

    init {
        var foundArea: Area? = null
        var mode = WebMode.TEMPLATE
        var dataMethod: Method? = null
        var dataTarget: Any? = null

        val scanResult = ClassGraph()
            .enableAnnotationInfo()
            .acceptPackages(scanPackage)
            .scan()

        // Find @MudArea class
        for (classInfo in scanResult.getClassesWithAnnotation(MudArea::class.java)) {
            val clazz = classInfo.loadClass()
            val annotation = clazz.getAnnotation(MudArea::class.java)
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Area) {
                foundArea = instance
                mode = annotation.webMode
                // Find @WebData method
                for (m in clazz.methods) {
                    if (m.isAnnotationPresent(WebData::class.java)) {
                        dataMethod = m
                        dataTarget = instance
                        break
                    }
                }
            }
            break // only one @MudArea per area
        }

        area = foundArea ?: Area()
        webMode = mode
        webDataMethod = dataMethod
        webDataTarget = dataTarget

        // Scan and register rooms
        for (classInfo in scanResult.getClassesWithAnnotation(MudRoom::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Room) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerRoom(key, instance)
                logger.info("Registered room: {}", key)
            }
        }

        // Scan and register items
        for (classInfo in scanResult.getClassesWithAnnotation(MudItem::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Item) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerItem(key, instance)
                logger.info("Registered item: {}", key)
            }
        }

        // Scan and register NPCs
        for (classInfo in scanResult.getClassesWithAnnotation(MudNPC::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is NPC) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerNPC(key, instance)
                logger.info("Registered NPC: {}", key)
            }
        }

        // Scan and register daemons
        for (classInfo in scanResult.getClassesWithAnnotation(MudDaemon::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Daemon) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerDaemon(key, instance)
                logger.info("Registered daemon: {}", key)
            }
        }

        scanResult.close()
    }

    @Suppress("UNCHECKED_CAST")
    fun getWebData(): Map<String, Any>? {
        val method = webDataMethod ?: return null
        val target = webDataTarget ?: return null
        return try {
            method.invoke(target) as? Map<String, Any>
        } catch (e: Exception) {
            logger.error("Failed to invoke @WebData method", e)
            null
        }
    }
}
