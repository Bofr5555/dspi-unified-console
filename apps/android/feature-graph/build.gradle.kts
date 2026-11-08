// :feature-graph — Wave B package GRAPH (docs/PACKAGES.md).
// Ports: GraphView.swift, GraphSpectrumOverlay.swift, PeqGraphEditor*.swift (logic from Windows PeqGraphEditor.cs), PeqBandHUD.swift
// Depends on contracts only: :bridge (DspiBridge) and :ui-common.
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.feature.graph"
    // -- WP CG30 -- Robolectric reads the merged manifest and resources (as :feature-settings, AN9)
    testOptions { unitTests.isIncludeAndroidResources = true }
    // -- WP AN20 -- ui-common's AtfChecks (the Accessibility Test Framework over Compose's nodes)
    sourceSets["test"].kotlin.srcDir("../ui-common/src/a11yTest/kotlin")
}

// -- WP CG30 -- the Android runtime Robolectric runs on, resolved (checksum-pinned) so it stays offline (as :app)
val robolectricRuntime: Configuration by configurations.creating

dependencies {
    implementation(project(":bridge"))
    implementation(project(":ui-common"))
    // @Previews (src/debug only) run against FakeDspiCore; release builds never see it.
    debugImplementation(project(":fake-core"))
    testImplementation(project(":fake-core"))
    testImplementation(libs.kotlinx.coroutines.test)
    // -- WP CG30 -- the overlay following over two real cores and crates/dspi-link's fake hub (as :app's AN34 test)
    testImplementation(project(":ffi"))
    testImplementation("${libs.jna.get().module}:${libs.versions.jna.get()}")
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    // -- WP AN20 --
    testImplementation(libs.compose.ui.test.junit4.accessibility)
    testImplementation(libs.robolectric)
    debugImplementation(platform(libs.compose.bom))
    debugImplementation(libs.compose.ui.test.manifest)
    robolectricRuntime(libs.robolectric.android.all)
    // -- WP CG30 -- the version :app resolves (and verification-metadata.xml pins), as :feature-settings
    constraints { testImplementation("androidx.tracing:tracing:1.2.0") }
}

// -- WP CG30 -- as :app: Robolectric offline, the host build of libdspi_ffi and the fake hub binary
tasks.withType<Test>().configureEach {
    val runtime = robolectricRuntime
    inputs.files(runtime)
    systemProperty("robolectric.offline", "true")
    doFirst { systemProperty("robolectric.dependency.dir", runtime.singleFile.parentFile.path) }
    dependsOn(":ffi:cargoBuildHost", ":link:cargoBuildFakeHub")
    val workspaceTarget = rootProject.layout.projectDirectory.dir("../../target").asFile.path
    val cargoTarget = providers.environmentVariable("CARGO_TARGET_DIR").getOrElse(workspaceTarget)
    systemProperty("jna.library.path", "$cargoTarget/debug")
    systemProperty("java.library.path", "$cargoTarget/debug")
    systemProperty("dspi.fakehub", "$cargoTarget/debug/dspi-link-fakehub")
}
