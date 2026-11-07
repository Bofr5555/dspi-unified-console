// :feature-onboarding — WP AN19: Getting Started and What's New.
// Ports: Onboarding/GettingStartedView.swift, Onboarding/WhatsNewView.swift (words and data from the core, CG10).
// Depends on contracts only: :ui-common (AppSettings, routes). Firmware versions and the
// connected board's match come from :app, which owns the one edge to :feature-firmware.
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.feature.onboarding"
}

dependencies {
    implementation(project(":ui-common"))
    // BackHandler: system back steps back through the wizard's stages.
    implementation(libs.androidx.activity.compose)
}
