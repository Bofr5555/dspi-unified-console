// :feature-settings — Wave B package SETTINGS (docs/PACKAGES.md).
// Ports: DSPi_ConsoleApp.swift SettingsView + *SettingsTab, SettingsSaveCoordinator; Windows PendingChangeTracker/PinNavigation
// Depends on contracts only: :bridge (DspiBridge) and :ui-common.
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.feature.settings"
    // -- WP AN9 -- Robolectric reads the merged manifest and resources (as :app, AN27)
    testOptions { unitTests.isIncludeAndroidResources = true }
}

// -- WP AN9 -- the Android runtime Robolectric runs on, resolved (checksum-pinned) so it stays offline (as :app)
val robolectricRuntime: Configuration by configurations.creating

dependencies {
    implementation(project(":bridge"))
    implementation(project(":ui-common"))
    // BackHandler: system back closes a Settings page before leaving Settings.
    implementation(libs.androidx.activity.compose)
    // @Previews run against FakeDspiCore (src/debug only; never in release).
    debugImplementation(project(":fake-core"))
    testImplementation(project(":fake-core"))
    testImplementation(libs.kotlinx.coroutines.test)
    // -- WP AN9 -- the pages on the real core's demo device (host libdspi_ffi over JNA), as :app's AN28 tests
    testImplementation(project(":ffi"))
    testImplementation("${libs.jna.get().module}:${libs.versions.jna.get()}")
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.robolectric)
    debugImplementation(platform(libs.compose.bom))
    debugImplementation(libs.compose.ui.test.manifest)
    robolectricRuntime(libs.robolectric.android.all)
    // -- WP AN9 -- the version :app resolves (and verification-metadata.xml pins); alone this classpath picks 1.1.0
    constraints { testImplementation("androidx.tracing:tracing:1.2.0") }
}

// -- WP AN9 -- as :app: Robolectric offline, and the host build of libdspi_ffi
tasks.withType<Test>().configureEach {
    val runtime = robolectricRuntime
    inputs.files(runtime)
    systemProperty("robolectric.offline", "true")
    doFirst { systemProperty("robolectric.dependency.dir", runtime.singleFile.parentFile.path) }
    dependsOn(":ffi:cargoBuildHost")
    val workspaceTarget = rootProject.layout.projectDirectory.dir("../../target").asFile.path
    val cargoTarget = providers.environmentVariable("CARGO_TARGET_DIR").getOrElse(workspaceTarget)
    systemProperty("jna.library.path", "$cargoTarget/debug")
    systemProperty("java.library.path", "$cargoTarget/debug")
}
