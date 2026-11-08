// :feature-analysis — Wave B package ANALYSIS (docs/PACKAGES.md).
// Ports: TestSignalsView.swift, SpectrumAnalyser*.swift, StatsView.swift, InterruptMonitor.swift
// Depends on contracts only: :bridge (DspiBridge) and :ui-common.
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.feature.analysis"
    // WP AN17: the fake-hub test runs the real bridge, which may touch android.util.Log
    testOptions { unitTests.isReturnDefaultValues = true }
    // -- WP AN20 -- Robolectric reads the merged manifest and resources (as :feature-settings, AN9)
    testOptions { unitTests.isIncludeAndroidResources = true }
    // -- WP AN20 -- ui-common's AtfChecks (the Accessibility Test Framework over Compose's nodes)
    sourceSets["test"].kotlin.srcDir("../ui-common/src/a11yTest/kotlin")
}

// -- WP AN20 -- the Android runtime Robolectric runs on, resolved (checksum-pinned) so it stays offline (as :app)
val robolectricRuntime: Configuration by configurations.creating

dependencies {
    implementation(project(":bridge"))
    implementation(project(":ui-common"))
    testImplementation(project(":fake-core"))
    // E2: @Previews on FakeDspiCore (debug source set only; never in release)
    debugImplementation(project(":fake-core"))
    testImplementation(libs.kotlinx.coroutines.test)
    // WP AN17: following over the real core and crates/dspi-link's fake hub (as :link's AN3 test)
    testImplementation(project(":ffi"))
    testImplementation("${libs.jna.get().module}:${libs.versions.jna.get()}")
    // -- WP AN20 -- Compose semantics tests and the Accessibility Test Framework checks, in Robolectric
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.compose.ui.test.junit4.accessibility)
    testImplementation(libs.robolectric)
    debugImplementation(platform(libs.compose.bom))
    debugImplementation(libs.compose.ui.test.manifest)
    robolectricRuntime(libs.robolectric.android.all)
    constraints { testImplementation("androidx.tracing:tracing:1.2.0") }
}

// WP AN17: the fake hub binary comes from :link's build task; the host core from :ffi's
val workspaceDir = rootProject.layout.projectDirectory.dir("../..")
val cargoTarget = providers.environmentVariable("CARGO_TARGET_DIR").getOrElse(workspaceDir.dir("target").asFile.path)

tasks.withType<Test>().configureEach {
    dependsOn(":link:cargoBuildFakeHub", ":ffi:cargoBuildHost")
    systemProperty("jna.library.path", "$cargoTarget/debug")
    systemProperty("java.library.path", "$cargoTarget/debug")
    systemProperty("dspi.fakehub", "$cargoTarget/debug/dspi-link-fakehub")
}

// -- WP AN20 -- as :app: Robolectric offline
tasks.withType<Test>().configureEach {
    val runtime = robolectricRuntime
    inputs.files(runtime)
    systemProperty("robolectric.offline", "true")
    doFirst { systemProperty("robolectric.dependency.dir", runtime.singleFile.parentFile.path) }
}
