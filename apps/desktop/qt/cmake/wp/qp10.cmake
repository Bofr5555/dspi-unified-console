# -- WP QP10 -- Settings window: shell, search, About, Advanced, Graphing,
# Spectrum Analyser, Updates and Overview pages
list(APPEND DSPI_APP_SOURCES
    src/controllers/SettingsController.h src/controllers/SettingsController.cpp
    src/controllers/PrefsMap.h src/controllers/PrefsMap.cpp
    src/controllers/PinMap.h src/controllers/PinMap.cpp
    src/controllers/AboutController.h src/controllers/AboutController.cpp
    src/render/PinGrid.h src/render/PinGrid.cpp)
list(APPEND DSPI_QML_FILES
    qml/settings/SettingsPanel.qml   # -- WP LAY1b -- was SettingsWindow.qml
    qml/settings/SettingsSearch.qml
    qml/settings/SettingsSidebarRow.qml
    qml/settings/SettingsPage.qml
    qml/settings/SettingsGroup.qml
    qml/settings/SettingsToggle.qml
    qml/settings/SettingsValueRow.qml
    qml/settings/SettingsSliderRow.qml
    qml/settings/SettingsPickerRow.qml
    qml/settings/SettingsExtensions.qml
    qml/settings/NoticeViewer.qml
    qml/settings/pages/AboutPage.qml
    qml/settings/pages/AdvancedPage.qml
    qml/settings/pages/GraphingPage.qml
    qml/settings/pages/SpectrumPage.qml
    qml/settings/pages/UpdatesPage.qml
    qml/settings/pages/OverviewPage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeSettings.cpp)

# A development build's copy of the notices the packages stage
# (packaging/licences/stage.sh), at notices/ beside the executables, so the
# About page lists them before any packaging ran. Packages carry their own.
set(DSPI_DEV_NOTICES "${CMAKE_CURRENT_BINARY_DIR}/notices")
configure_file("${DSPI_REPO}/NOTICE.md" "${DSPI_DEV_NOTICES}/NOTICE.txt" COPYONLY)
foreach(notice GPL-3.0.txt LGPL-3.0.txt TERMS.txt rust-crates.txt)
    configure_file("${CMAKE_CURRENT_SOURCE_DIR}/../packaging/licences/${notice}" "${DSPI_DEV_NOTICES}/LICENSES/${notice}" COPYONLY)
endforeach()
foreach(notice firmware-THIRD-PARTY-NOTICES.txt AutoEq-MIT.txt)
    configure_file("${DSPI_REPO}/assets/licences/${notice}" "${DSPI_DEV_NOTICES}/LICENSES/${notice}" COPYONLY)
endforeach()
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/icons/lucide/LICENSE" "${DSPI_DEV_NOTICES}/LICENSES/Lucide/LICENSE" COPYONLY)
list(APPEND DSPI_SMOKE_QML_FILES smoke/SmokeSettingsPage.qml smoke/SmokeSettingsSection.qml smoke/SmokeSettingsFooter.qml)
