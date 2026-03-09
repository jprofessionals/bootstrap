plugins {
    kotlin("jvm")
    application
}

application {
    mainClass.set("mud.launcher.MainKt")
}

dependencies {
    implementation(project(":mud-mop-jvm"))
    implementation(project(":stdlib"))
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    implementation("ch.qos.logback:logback-classic:1.5.12")
    implementation("com.kohlschutter.junixsocket:junixsocket-core:2.10.1")

    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
    testImplementation("com.kohlschutter.junixsocket:junixsocket-core:2.10.1")
}

tasks.jar {
    archiveFileName.set("launcher.jar")
    manifest { attributes("Main-Class" to "mud.launcher.MainKt") }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from(configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) })
}
