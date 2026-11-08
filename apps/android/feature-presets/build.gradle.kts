// :feature-presets — Wave B package PRESETS (docs/PACKAGES.md).
// Ports: ContentView preset menu, Unsaved Changes, PresetDocument*/FilterFile import-export (core JSON), AutoEQ/*
// Depends on contracts only: :bridge (DspiBridge) and :ui-common.
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.feature.presets"
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
    // Catalog libraries (no new artifacts): FileProvider for the share sheet.
    implementation(libs.androidx.core.ktx)
    // E9: the controller lives in a ViewModel (catalog library, already verified).
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    // @Previews (src/debug) run against FakeDspiCore, as PACKAGES.md asks; never in release.
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
