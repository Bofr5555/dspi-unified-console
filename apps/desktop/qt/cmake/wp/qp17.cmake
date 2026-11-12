# -- WP QP17 -- Onboarding, About and updates: Getting Started, What's New,
# the Help menu's rows, the About licence block, the update checker and banner
list(APPEND DSPI_APP_SOURCES
    src/controllers/OnboardingController.h src/controllers/OnboardingController.cpp
    src/platform/AppUpdater.h src/platform/AppUpdater.cpp)
list(APPEND DSPI_QML_FILES
    qml/onboarding/GettingStarted.qml
    qml/onboarding/StepDots.qml
    qml/onboarding/InstallCard.qml
    qml/onboarding/InfoRow.qml
    qml/onboarding/WhatsNewWindow.qml
    qml/onboarding/HelpHost.qml
    qml/onboarding/AboutLicenceSection.qml
    qml/onboarding/OnboardingDeveloperSection.qml
    qml/banners/UpdateBanner.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeOnboarding.cpp smoke/SmokeUpdates.cpp)

# The release feed the update checker asks (behaviour.md §17): https only.
# Empty, the default, is a build that never checks: developer builds and every
# distribution package. A release build passes the project's feed.
set(DSPI_UPDATE_FEED_URL "" CACHE STRING "Release feed of the update checker (https); empty: never check")
if(DSPI_UPDATE_FEED_URL AND NOT DSPI_UPDATE_FEED_URL MATCHES "^https://")
    message(FATAL_ERROR "DSPI_UPDATE_FEED_URL must be an https URL")
endif()
set_property(SOURCE src/platform/AppUpdater.cpp APPEND PROPERTY
    COMPILE_DEFINITIONS "DSPI_UPDATE_FEED_URL=\"${DSPI_UPDATE_FEED_URL}\"")

# The firmware this build ships, from its bundled images' names
# (firmware/DSPi-<chip>-v<version>.uf2): What's New and the wizard's board step
file(GLOB DSPI_QP17_IMAGES RELATIVE "${CMAKE_CURRENT_SOURCE_DIR}/firmware"
    "${CMAKE_CURRENT_SOURCE_DIR}/firmware/DSPi-*-v*.uf2")
set(DSPI_QP17_FIRMWARE "")
foreach(image IN LISTS DSPI_QP17_IMAGES)
    if(image MATCHES "-v([0-9]+\\.[0-9]+\\.[0-9]+(-beta[0-9]+)?)\\.uf2$")
        set(DSPI_QP17_FIRMWARE "${CMAKE_MATCH_1}")
    endif()
endforeach()
set_property(SOURCE src/controllers/OnboardingController.cpp APPEND PROPERTY
    COMPILE_DEFINITIONS "DSPI_BUNDLED_FIRMWARE=\"${DSPI_QP17_FIRMWARE}\"")

# Help > Save Diagnostics... needs the core's dspi_diagnostics_save (P0E); a
# core without it builds the app with the row hidden
file(STRINGS "${DSPI_CAPI_INCLUDE}/dspi_core.h" DSPI_QP17_DIAG REGEX "dspi_diagnostics_save\\(")
if(DSPI_QP17_DIAG)
    set_property(SOURCE src/controllers/OnboardingController.cpp APPEND PROPERTY
        COMPILE_DEFINITIONS DSPI_HAVE_DIAGNOSTICS)
endif()

# The checker's requests
list(APPEND DSPI_QT_COMPONENTS Network)
list(APPEND DSPI_APP_LIBS Qt6::Network)
