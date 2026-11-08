// :link — DSPi Link client on Android: hub discovery (NsdManager, manual
// addresses), the session engine (connect, PIN pairing, device choice, following
// the core's supervisor), Keystore-encrypted tokens, the Link settings.
// The protocol itself is Rust (crates/dspi-link via dspi-ffi); this module
// drives it through DspiCoreApi on the REMOTE stack's own core and bridge.
plugins {
    id("dspi.android-library")
}

android {
    namespace = "se.bofr.dspi.link"
    testOptions { unitTests.isReturnDefaultValues = true }
}

dependencies {
    api(project(":bridge"))
    implementation(libs.kotlinx.coroutines.android)
    testImplementation(project(":fake-core"))
    testImplementation(libs.kotlinx.coroutines.test)
    // The engine against the real core and crates/dspi-link's fake hub (WP AN3)
    testImplementation(project(":ffi"))
    testImplementation("${libs.jna.get().module}:${libs.versions.jna.get()}")
}

// The fake hub as a process: faults go in on its stdin, as tools/link-fakehub.sh does by hand.
val workspaceDir = rootProject.layout.projectDirectory.dir("../..")
val cargoTarget = providers.environmentVariable("CARGO_TARGET_DIR").getOrElse(workspaceDir.dir("target").asFile.path)

val cargoBuildFakeHub by tasks.registering(Exec::class) {
    description = "Build dspi-link-fakehub for the host, for the engine's tests."
    workingDir = workspaceDir.asFile
    commandLine("cargo", "build", "-p", "dspi-link", "--features", "fake-hub", "--bin", "dspi-link-fakehub", "--locked")
    doFirst {
        check(System.getenv("DSPI_IN_CONTAINER") == "1") {
            "The fake hub's Rust build runs inside the build container: use tools/dev.sh ./gradlew ..."
        }
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(cargoBuildFakeHub, ":ffi:cargoBuildHost")
    systemProperty("jna.library.path", "$cargoTarget/debug")
    systemProperty("java.library.path", "$cargoTarget/debug")
    systemProperty("dspi.fakehub", "$cargoTarget/debug/dspi-link-fakehub")
    // -- WP LK4 -- the core's first dial retries for 60 s; 2 s here, so a test of a
    // first dial that fails sees it fail (DSPI_LINK_FIRST_DIAL_MS, dspi-core link.rs)
    environment("DSPI_LINK_FIRST_DIAL_MS", "2000")
}
