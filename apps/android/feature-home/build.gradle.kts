// :feature-home — Wave B package HOME (docs/PACKAGES.md).
// Ports: ContentView.swift (HomeScreen/ChannelScreen), Components.swift, DashboardView.swift, CustomSlider/ChannelPalette
// Depends on contracts only: :bridge (DspiBridge) and :ui-common.
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.feature.home"
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
    // Allowed feature edges: the embeddable GraphView (GRAPH contract) …
    implementation(project(":feature-graph"))
    // Coordinator F1: the second allowed feature edge — PRESETS' preset flows
    // (PresetMenuActions, AlertHost, ConsoleAlertDialog) behind the sidebar picker.
    implementation(project(":feature-presets"))
    // -- WP AN34 -- the analyser's bar strip (AN17's SpectrumBarStrip) above the cards and channel details
    implementation(project(":feature-analysis"))
    // E9: the preset controller and its alert slot live in a ViewModel.
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    // Coordinator E2: @Previews run on FakeDspiCore (debug only; release never sees it).
    debugImplementation(project(":fake-core"))
    testImplementation(project(":fake-core"))
    testImplementation(libs.kotlinx.coroutines.test)
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

// -- WP AN20 -- as :app: Robolectric offline
tasks.withType<Test>().configureEach {
    val runtime = robolectricRuntime
    inputs.files(runtime)
    systemProperty("robolectric.offline", "true")
    doFirst { systemProperty("robolectric.dependency.dir", runtime.singleFile.parentFile.path) }
}
