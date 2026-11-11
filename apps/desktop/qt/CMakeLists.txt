# The Qt desktop app: Qt 6.8 or newer, Qt Quick with the Basic style, over the
# core's C ABI (crates/dspi-capi, header include/dspi_core.h).
#
#   cmake -S apps/desktop/qt -B build -G Ninja -DDSPI_CARGO_TARGET_DIR=<cargo target dir>
#
# Options:
#   DSPI_CARGO_TARGET_DIR  where cargo built dspi-capi (default: the repo's target/)
#   DSPI_RUST_TARGET       the Rust target triple of a cross build (empty: the host's)
#   DSPI_SMOKE             also build DSPiConsoleSmoke, the app with the offscreen
#                          smoke driver (smoke/, ci/check.sh --step qt-offscreen)
#
# Every QML file, C++ type and icon belongs to one QML module, UConsole. A work
# package adds its files to the lists below under its own marker
# (# -- WP QPn --); nothing else in this file needs to change.
cmake_minimum_required(VERSION 3.21)
project(DSPiConsole VERSION 0.13.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

# Kept so the packaging scripts' invocations still configure; only 6 is built.
set(DSPI_QT_MAJOR 6 CACHE STRING "Qt major version (6 only)")
if(NOT DSPI_QT_MAJOR STREQUAL "6")
    message(FATAL_ERROR "DSPI_QT_MAJOR=${DSPI_QT_MAJOR}: this app builds with Qt 6.8 or newer only")
endif()

get_filename_component(DSPI_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
set(DSPI_CARGO_TARGET_DIR "${DSPI_REPO}/target" CACHE PATH "cargo's target directory")
set(DSPI_RUST_TARGET "" CACHE STRING "Rust target triple of a cross build")
option(DSPI_SMOKE "Build DSPiConsoleSmoke, the app with its smoke driver" OFF)

find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Widgets Qml Quick QuickControls2 Svg)
find_package(Qt6 6.8 QUIET COMPONENTS LinguistTools)
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    find_package(Qt6 6.8 REQUIRED COMPONENTS DBus)
endif()
qt_standard_project_setup(REQUIRES 6.8)
qt_policy(SET QTP0001 NEW)

# ---- The core (dspi-capi) -------------------------------------------------
if(DSPI_RUST_TARGET)
    set(DSPI_CAPI_DIR "${DSPI_CARGO_TARGET_DIR}/${DSPI_RUST_TARGET}/release")
else()
    set(DSPI_CAPI_DIR "${DSPI_CARGO_TARGET_DIR}/release")
endif()
set(DSPI_CAPI_INCLUDE "${DSPI_REPO}/crates/dspi-capi/include")
if(NOT EXISTS "${DSPI_CAPI_INCLUDE}/dspi_core.h")
    message(FATAL_ERROR "${DSPI_CAPI_INCLUDE}/dspi_core.h is missing: build dspi-capi first (cargo build --release -p dspi-capi)")
endif()

add_library(dspi_capi UNKNOWN IMPORTED)
set_target_properties(dspi_capi PROPERTIES INTERFACE_INCLUDE_DIRECTORIES "${DSPI_CAPI_INCLUDE}")
if(WIN32)
    # The DLL, next to the executable (packaging/windows/deploy-mingw.sh takes it from there)
    set_target_properties(dspi_capi PROPERTIES IMPORTED_LOCATION "${DSPI_CAPI_DIR}/dspi_capi.dll.a")
    set(DSPI_CAPI_DLL "${DSPI_CAPI_DIR}/dspi_capi.dll")
else()
    # The static library and what it needs from the system (cargo's native-static-libs)
    set_target_properties(dspi_capi PROPERTIES IMPORTED_LOCATION "${DSPI_CAPI_DIR}/libdspi_capi.a")
    if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
        set_property(TARGET dspi_capi PROPERTY INTERFACE_LINK_LIBRARIES asound util rt pthread m dl)
    endif()
endif()

# ---- Sources -------------------------------------------------------------
set(DSPI_APP_SOURCES
    src/AppIdentity.h
    src/core/CoreApi.h
    src/core/Core.h src/core/Core.cpp
    src/core/CoreWorker.h src/core/CoreWorker.cpp
    src/core/Snapshot.h src/core/Snapshot.cpp
    src/core/DeviceModel.h src/core/DeviceModel.cpp
    src/core/ChannelListModel.h src/core/ChannelListModel.cpp
    src/core/BandListModel.h src/core/BandListModel.cpp
    src/core/PresetModel.h src/core/PresetModel.cpp
    src/controllers/Registrar.h src/controllers/Registrar.cpp
    src/controllers/Shell.h src/controllers/Shell.cpp
    src/platform/IconProvider.h src/platform/IconProvider.cpp
    src/platform/TokenStore.h src/platform/TokenStore.cpp
)
# Kept from the previous tree, not built until their work package adapts them to
# Core and DeviceModel (README.md, "Parked files"):
#   src/controllers/LinkController.h        QP13
#   src/platform/SystemIntegration.{h,cpp}  QP14
#   src/platform/FirewallController.cpp     QP14
#   src/platform/HostAudioFormatWindows.cpp QP18

set(DSPI_QML_FILES
    qml/Theme/Theme.qml
    qml/Theme/ChannelPalette.qml
    qml/components/Icon.qml
    qml/components/IconButton.qml
    qml/components/CustomSlider.qml
    qml/components/BorderedField.qml
    qml/components/Pill.qml
    qml/components/Card.qml
    qml/components/PopoverPanel.qml
    qml/components/Banner.qml
    qml/components/HelpTip.qml
    qml/components/AppMenu.qml
    qml/components/AppMenuItem.qml
    qml/components/AppMenuBar.qml
    qml/components/ToolWindow.qml
    qml/main/MainWindow.qml
    qml/main/MenuTable.qml
    qml/main/Menus.qml
    qml/main/BannerHost.qml
    qml/main/Sidebar.qml
    qml/main/RightPane.qml
    qml/main/DeviceStatus.qml
)
set_source_files_properties(qml/Theme/Theme.qml qml/Theme/ChannelPalette.qml qml/main/MenuTable.qml
    PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

# The icons: the files icons/lucide/manifest.txt names, fetched by
# icons/fetch-lucide.sh, plus any of our own in icons/own/.
file(STRINGS icons/lucide/manifest.txt DSPI_ICON_LINES REGEX "^[a-z0-9-]+")
set(DSPI_ICON_FILES icons/lucide/LICENSE)
foreach(line IN LISTS DSPI_ICON_LINES)
    string(REGEX MATCH "^[a-z0-9-]+" name "${line}")
    list(APPEND DSPI_ICON_FILES "icons/lucide/${name}.svg")
endforeach()
file(GLOB DSPI_OWN_ICONS RELATIVE "${CMAKE_CURRENT_SOURCE_DIR}" icons/own/*.svg)
list(APPEND DSPI_ICON_FILES ${DSPI_OWN_ICONS})

set(DSPI_SMOKE_SOURCES
    smoke/SmokeDriver.h smoke/SmokeDriver.cpp
    smoke/SmokeProbe.h smoke/SmokeBase.cpp
)
set(DSPI_SMOKE_QML_FILES
    smoke/SmokeProbeBanner.qml
    smoke/SmokeProbeWindow.qml
)

# ---- Targets -------------------------------------------------------------
# One function for the app and the smoke build, so both are the same app.
function(dspi_add_app target)
    qt_add_executable(${target} WIN32 src/main.cpp ${DSPI_APP_SOURCES} ${ARGN})
    qt_add_qml_module(${target}
        URI UConsole
        VERSION 1.0
        OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/${target}_qml/UConsole"
        QML_FILES ${DSPI_QML_FILES}
        RESOURCES ${DSPI_ICON_FILES}
    )
    target_include_directories(${target} PRIVATE src src/core src/controllers src/platform)
    target_compile_definitions(${target} PRIVATE DSPI_APP_VERSION="${PROJECT_VERSION}")
    target_link_libraries(${target} PRIVATE
        Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg dspi_capi)
    if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
        target_link_libraries(${target} PRIVATE Qt6::DBus)
    endif()
    if(WIN32)
        target_link_libraries(${target} PRIVATE advapi32 credui ole32 oleaut32)
        add_custom_command(TARGET ${target} POST_BUILD
            COMMAND ${CMAKE_COMMAND} -E copy_if_different "${DSPI_CAPI_DLL}" "$<TARGET_FILE_DIR:${target}>")
    endif()
endfunction()

dspi_add_app(DSPiConsole)

if(DSPI_SMOKE)
    find_package(Qt6 6.8 REQUIRED COMPONENTS Test)
    dspi_add_app(DSPiConsoleSmoke ${DSPI_SMOKE_SOURCES})
    qt_target_qml_sources(DSPiConsoleSmoke QML_FILES ${DSPI_SMOKE_QML_FILES})
    target_compile_definitions(DSPiConsoleSmoke PRIVATE DSPI_SMOKE)
    target_include_directories(DSPiConsoleSmoke PRIVATE smoke)
    target_link_libraries(DSPiConsoleSmoke PRIVATE Qt6::Test)
endif()

# ---- Translations (English only) ------------------------------------------
# The .ts is generated from the sources; ci/check.sh --step qt-ts compares the
# committed file with what lupdate writes (the same arguments as here).
if(TARGET Qt6::lupdate)
    add_custom_target(dspi_lupdate
        COMMAND Qt6::lupdate -silent -locations none -no-obsolete -source-language en -target-language en
                qml src -ts translations/dspi-uconsole_en.ts
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
        COMMENT "Regenerating translations/dspi-uconsole_en.ts"
        VERBATIM)
endif()
