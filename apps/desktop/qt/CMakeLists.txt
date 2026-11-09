cmake_minimum_required(VERSION 3.16)
# macOS: the oldest release the app runs on (Qt 6.11's own minimum); the
# Rust core is built for the same (MACOSX_DEPLOYMENT_TARGET, see
# packaging/macos/build-macos.sh)
set(CMAKE_OSX_DEPLOYMENT_TARGET "13.0" CACHE STRING "Oldest macOS the app runs on")
project(DSPiConsole VERSION 0.13.0 LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

# Qt 6 when it is found, else Qt 5.15. DSPI_QT_MAJOR=5 or 6 forces one.
set(DSPI_QT_MAJOR "" CACHE STRING "Qt major version to build against (5 or 6; empty: Qt 6 if found)")
if(APPLE AND NOT DSPI_QT_MAJOR STREQUAL "6")
    # Homebrew's Qt 5, when Qt 5 is asked for or Qt 6 isn't installed
    list(APPEND CMAKE_PREFIX_PATH "/opt/homebrew/opt/qt@5")
endif()
if(DSPI_QT_MAJOR)
    set(_qt_names Qt${DSPI_QT_MAJOR})
else()
    set(_qt_names Qt6 Qt5)
endif()
find_package(QT NAMES ${_qt_names} REQUIRED COMPONENTS Core)
if(QT_VERSION_MAJOR EQUAL 5)
    find_package(Qt5 5.15 REQUIRED COMPONENTS Core Gui Widgets Quick QuickControls2 Qml Network)
else()
    # -- WP QD21c -- 6.5: on Qt 6.4 (6.4.2, 6.4.3) the app crashes at its first window activation
    find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Widgets Quick QuickControls2 Qml Network)
endif()
message(STATUS "Building against Qt ${QT_VERSION}")
# -- WP QD8 -- Linux keeps Link tokens through the Secret Service, over D-Bus
if(UNIX AND NOT APPLE)
    find_package(Qt${QT_VERSION_MAJOR} REQUIRED COMPONENTS DBus)
endif()

# The Rust core (dspi-core). Build it first with cargo; these say where its
# header and library are. DSPI_RUST_TARGET is the cargo --target triple when
# cross-compiling (e.g. x86_64-pc-windows-gnu), empty for a native build.
# The core is the C ABI crate crates/dspi-capi (library dspi_capi, header
# dspi_core.h); cargo's target directory is the repository root's target/.
get_filename_component(DSPI_REPO_ROOT "${CMAKE_SOURCE_DIR}/../../.." ABSOLUTE)
set(DSPI_CORE_DIR "${DSPI_REPO_ROOT}/crates/dspi-capi" CACHE PATH "Directory holding include/dspi_core.h")
set(DSPI_CARGO_TARGET_DIR "${DSPI_REPO_ROOT}/target" CACHE PATH "cargo's target directory")
set(DSPI_RUST_TARGET "" CACHE STRING "cargo --target triple, empty for the host")
set(DSPI_CORE_LIB "dspi_capi" CACHE STRING "Rust core library name")
if(DSPI_RUST_TARGET)
    set(DSPI_CORE_LIBDIR "${DSPI_CARGO_TARGET_DIR}/${DSPI_RUST_TARGET}/release")
else()
    set(DSPI_CORE_LIBDIR "${DSPI_CARGO_TARGET_DIR}/release")
endif()

# The version the app reports, e.g. 0.13.0 (or 0.14.0-beta1)
file(STRINGS ${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp _ver REGEX "setApplicationVersion\\(\"[^\"]+\"\\)")
string(REGEX REPLACE ".*setApplicationVersion\\(\"([^\"]+)\"\\).*" "\\1" DSPI_VERSION "${_ver}")
string(REGEX MATCH "^([0-9]+)\\.([0-9]+)\\.([0-9]+)" _ "${DSPI_VERSION}")
set(DSPI_VER_MAJOR ${CMAKE_MATCH_1})
set(DSPI_VER_MINOR ${CMAKE_MATCH_2})
set(DSPI_VER_PATCH ${CMAKE_MATCH_3})

set(DSPI_APP_SOURCES
    src/main.cpp
    src/DSPiBridge.cpp
    src/BodePlotItem.cpp
    src/PeqEditorItem.cpp
    src/RtaController.cpp
    src/RtaViews.cpp
    src/PointerTracker.cpp
    src/SiggenController.cpp
    src/CsController.cpp
    src/NamesController.cpp # -- WP QD4 --
    src/OutputLatencyController.cpp # -- WP QD5 --
    src/FirmwareUpdater.cpp
    src/AutoEqLibrary.cpp
    src/StatsController.cpp
    src/MonitorModel.cpp
    src/ConfigFiles.cpp
    src/MeterItem.cpp
    src/WindowEffects.cpp
    # -- WP QD2 --
    src/ExtController.cpp
    # -- WP QD8 --
    src/LinkController.cpp
    # -- WP QD32 --
    src/OnboardingText.cpp
    src/TokenStore.cpp
    # -- WP QD7 --
    src/DemoController.cpp
    # -- WP QD33 --
    src/AboutController.cpp
    # -- WP QD18 --
    src/HostAudioFormat.cpp
    src/HostAudioFormatLinux.cpp
    src/HostAudioFormatWindows.cpp
    # -- WP QD12 --
    src/TrayController.cpp
    # -- WP QD28 --
    src/VregController.cpp
    # -- WP QD14 --
    src/HubController.cpp
    # -- WP QD13 --
    src/SystemIntegration.cpp
    resources.qrc
)
# -- WP QD16 -- The check for a newer release (src/AppUpdater.cpp). Distro packages
# update the app themselves: they build with -DDSPI_UPDATE_CHECK=OFF, which leaves
# it out, page and all. The feed is a GitHub release list; empty until the owner
# publishes one (OD1), and then the app says it has none and never checks.
option(DSPI_UPDATE_CHECK "Build the opt-in check for a newer release (off for distro packages)" ON)
set(DSPI_UPDATE_FEED_URL "" CACHE STRING "Release list the update check asks (GitHub releases API URL)")
if(DSPI_UPDATE_CHECK)
    list(APPEND DSPI_APP_SOURCES src/AppUpdater.cpp)
endif()
# -- WP QD23 -- Translations. The QML's qsTr strings are collected by lupdate into
# translations/dspi-console_<lang>.ts (`cmake --build <dir> --target dspi_lupdate`
# refreshes them); lrelease builds each into a .qm in the resources under :/i18n,
# which main.cpp loads for the system's language. English only for now: the
# source text is English, so the English file carries no translations. Qt's own
# tools when their CMake package is there, else lrelease/lupdate on the PATH
# (the .qm format is the same for Qt 5 and 6). Without either the app builds
# untranslated, which is English.
set(DSPI_TS_FILES ${CMAKE_CURRENT_SOURCE_DIR}/translations/dspi-console_en.ts)
find_package(Qt${QT_VERSION_MAJOR}LinguistTools QUIET)
if(TARGET Qt${QT_VERSION_MAJOR}::lrelease)
    set(_dspi_lrelease Qt${QT_VERSION_MAJOR}::lrelease)
    set(_dspi_lupdate Qt${QT_VERSION_MAJOR}::lupdate)
else()
    find_program(DSPI_LRELEASE NAMES lrelease-qt${QT_VERSION_MAJOR} lrelease lrelease-qt6)
    find_program(DSPI_LUPDATE NAMES lupdate-qt${QT_VERSION_MAJOR} lupdate lupdate-qt6)
    set(_dspi_lrelease ${DSPI_LRELEASE})
    set(_dspi_lupdate ${DSPI_LUPDATE})
endif()
if(_dspi_lrelease)
    set(_dspi_i18n_dir ${CMAKE_CURRENT_BINARY_DIR}/i18n)
    set(_dspi_i18n_qrc "<RCC>\n    <qresource prefix=\"/i18n\">\n")
    set(_dspi_qm_files)
    foreach(_ts IN LISTS DSPI_TS_FILES)
        get_filename_component(_name ${_ts} NAME_WE)
        add_custom_command(OUTPUT ${_dspi_i18n_dir}/${_name}.qm
            COMMAND ${CMAKE_COMMAND} -E make_directory ${_dspi_i18n_dir}
            COMMAND ${_dspi_lrelease} -silent ${_ts} -qm ${_dspi_i18n_dir}/${_name}.qm
            DEPENDS ${_ts}
            VERBATIM)
        list(APPEND _dspi_qm_files ${_dspi_i18n_dir}/${_name}.qm)
        string(APPEND _dspi_i18n_qrc "        <file>${_name}.qm</file>\n")
    endforeach()
    string(APPEND _dspi_i18n_qrc "    </qresource>\n</RCC>\n")
    # Written only when it changes; rcc run here rather than by AUTORCC, so the
    # resource waits for the .qm files it holds
    file(WRITE ${_dspi_i18n_dir}/i18n.qrc.in "${_dspi_i18n_qrc}")
    configure_file(${_dspi_i18n_dir}/i18n.qrc.in ${_dspi_i18n_dir}/i18n.qrc COPYONLY)
    add_custom_command(OUTPUT ${_dspi_i18n_dir}/qrc_i18n.cpp
        COMMAND Qt${QT_VERSION_MAJOR}::rcc --name dspi_i18n -o ${_dspi_i18n_dir}/qrc_i18n.cpp ${_dspi_i18n_dir}/i18n.qrc
        DEPENDS ${_dspi_i18n_dir}/i18n.qrc ${_dspi_qm_files}
        WORKING_DIRECTORY ${_dspi_i18n_dir}
        VERBATIM)
    set_source_files_properties(${_dspi_i18n_dir}/qrc_i18n.cpp PROPERTIES SKIP_AUTOGEN ON)
    list(APPEND DSPI_APP_SOURCES ${_dspi_i18n_dir}/qrc_i18n.cpp)
    message(STATUS "Translations: ${_dspi_lrelease}")
else()
    message(WARNING "No lrelease found (Qt Linguist tools): the app builds without its translations")
endif()
if(_dspi_lupdate)
    # The .ts files from the sources; not part of the build. -- WP QD41 -- No
    # locations: the file holds the strings and their contexts only, so a line
    # moved in qml/ or src/ leaves it as it is (ci/check.sh --step qt-ts runs
    # the same arguments)
    add_custom_target(dspi_lupdate
        COMMAND ${_dspi_lupdate} -silent -locations none -no-obsolete -source-language en -target-language en
                qml src -ts ${DSPI_TS_FILES}
        WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
        VERBATIM)
endif()
# -- WP QD23 end --
# -- WP QD21a -- The QML in resources.qrc compiled to byte code at build time, so
# the first start does not compile it (it took several seconds on Windows). The
# sources stay in the resources: a unit that does not match the Qt it runs on is
# compiled from them as before. Qt 6: qmlcachegen per file, as qt_add_qml_module
# runs it, with no QML module; only byte code, as the engine would make itself.
# Qt 5: Qt Quick Compiler. The file list is read from resources.qrc.
option(DSPI_QML_PRECOMPILE "Compile the app's QML to byte code at build time" ON)
if(DSPI_QML_PRECOMPILE AND QT_VERSION_MAJOR EQUAL 5)
    find_package(Qt5QuickCompiler REQUIRED)
    set(QTQUICK_COMPILER_RETAINED_RESOURCES resources.qrc)
    qtquick_compiler_add_resources(_dspi_qml_cache resources.qrc)
    list(REMOVE_ITEM DSPI_APP_SOURCES resources.qrc)
    list(APPEND DSPI_APP_SOURCES ${_dspi_qml_cache})
elseif(DSPI_QML_PRECOMPILE)
    set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/resources.qrc)
    file(READ ${CMAKE_CURRENT_SOURCE_DIR}/resources.qrc _dspi_qrc)
    string(REGEX MATCHALL "<file( alias=\"[^\"]*\")?>[^<]*\\.(qml|js|mjs)</file>" _dspi_qrc_qml "${_dspi_qrc}")
    set(_dspi_qml_dir ${CMAKE_CURRENT_BINARY_DIR}/.rcc/dspi-qmlcache)
    set(_dspi_qml_paths)
    foreach(_entry IN LISTS _dspi_qrc_qml)
        string(REGEX REPLACE "^<file[^>]*>([^<]*)</file>$" "\\1" _file "${_entry}")
        set(_alias ${_file})
        if(_entry MATCHES "^<file alias=\"([^\"]+)\">")
            set(_alias ${CMAKE_MATCH_1})
        endif()
        string(REGEX REPLACE "[./]" "_" _out "${_alias}")
        add_custom_command(OUTPUT ${_dspi_qml_dir}/${_out}.cpp
            COMMAND ${CMAKE_COMMAND} -E make_directory ${_dspi_qml_dir}
            COMMAND Qt6::qmlcachegen --only-bytecode --resource-path /${_alias}
                    -o ${_dspi_qml_dir}/${_out}.cpp ${CMAKE_CURRENT_SOURCE_DIR}/${_file}
            DEPENDS Qt6::qmlcachegen ${CMAKE_CURRENT_SOURCE_DIR}/${_file}
            VERBATIM)
        list(APPEND DSPI_APP_SOURCES ${_dspi_qml_dir}/${_out}.cpp)
        list(APPEND _dspi_qml_paths /${_alias})
    endforeach()
    # The loader registers the units under their resource paths
    add_custom_command(OUTPUT ${_dspi_qml_dir}/qmlcache_loader.cpp
        COMMAND Qt6::qmlcachegen --resource-name qmlcache_dspi
                -o ${_dspi_qml_dir}/qmlcache_loader.cpp ${_dspi_qml_paths}
        DEPENDS Qt6::qmlcachegen ${CMAKE_CURRENT_SOURCE_DIR}/resources.qrc
        VERBATIM)
    list(APPEND DSPI_APP_SOURCES ${_dspi_qml_dir}/qmlcache_loader.cpp)
    set_source_files_properties(${_dspi_qml_dir}/qmlcache_loader.cpp PROPERTIES SKIP_AUTOGEN ON)
    list(LENGTH _dspi_qml_paths _n)
    message(STATUS "QML precompiled: ${_n} files")
endif()
add_executable(DSPiConsole ${DSPI_APP_SOURCES})

# The app with its offscreen test driver (ci/check.sh --step qt-offscreen).
option(DSPI_SMOKE "Also build DSPiConsoleSmoke, the app with its test driver" OFF)
if(DSPI_SMOKE)
    add_executable(DSPiConsoleSmoke ${DSPI_APP_SOURCES} smoke/SmokeDriver.cpp smoke/SmokeDriverHw.cpp smoke/SmokeDriverReplug.cpp smoke/SmokeDriverSeam.cpp smoke/SmokeDriverExt.cpp smoke/SmokeDriverNames.cpp smoke/SmokeDriverLatency.cpp smoke/SmokeDriverLink.cpp smoke/SmokeDriverLinkFault.cpp smoke/SmokeDriverDevices.cpp smoke/LinkFakeHub.h smoke/SmokeDriverReadOnly.cpp smoke/SmokeDriverDemo.cpp smoke/SmokeDriverRing.cpp smoke/SmokeDriverPoll.cpp smoke/SmokeDriverSaveBar.cpp smoke/SmokeDriverAbout.cpp smoke/SmokeDriverOnboarding.cpp smoke/SmokeDriverAutoEq.cpp smoke/SmokeDriverHostAudio.cpp smoke/SmokeDriverTray.cpp smoke/SmokeDriverFirmware.cpp smoke/SmokeDriverHub.cpp smoke/SmokeDriverVreg.cpp smoke/SmokeDriverShortcuts.cpp smoke/SmokeDriverRtaFollow.cpp smoke/SmokeDriverNetworking.cpp smoke/SmokeDriverSystem.cpp smoke/SmokeDriverDelay.cpp smoke/SmokeDriverSliderKeys.cpp smoke/SmokeDriverBug1.cpp smoke/SmokeDriver.h smoke/smoke.qrc)
    target_include_directories(DSPiConsoleSmoke PRIVATE ${DSPI_CORE_DIR}/include src/ smoke/)
    target_compile_definitions(DSPiConsoleSmoke PRIVATE DSPI_SMOKE)
    # -- WP QD23 -- The pseudo-locale pass
    target_sources(DSPiConsoleSmoke PRIVATE smoke/SmokeDriverPseudo.cpp)
    # -- WP QD16 --
    if(DSPI_UPDATE_CHECK)
        target_sources(DSPiConsoleSmoke PRIVATE smoke/SmokeDriverUpdates.cpp)
        target_compile_definitions(DSPiConsoleSmoke PRIVATE DSPI_UPDATE_CHECK DSPI_UPDATE_FEED_URL="${DSPI_UPDATE_FEED_URL}")
    endif()
    target_link_libraries(DSPiConsoleSmoke PRIVATE
        Qt::Core Qt::Gui Qt::Widgets Qt::Quick Qt::QuickControls2 Qt::Qml Qt::Network Qt::DBus
        ${DSPI_CORE_LIBDIR}/lib${DSPI_CORE_LIB}.a pthread dl m asound)
endif()

target_include_directories(DSPiConsole PRIVATE
    ${DSPI_CORE_DIR}/include
    src/
)

target_link_libraries(DSPiConsole PRIVATE
    Qt::Core Qt::Gui Qt::Widgets Qt::Quick Qt::QuickControls2 Qt::Qml Qt::Network
)
# -- WP QD16 --
if(DSPI_UPDATE_CHECK)
    target_compile_definitions(DSPiConsole PRIVATE DSPI_UPDATE_CHECK DSPI_UPDATE_FEED_URL="${DSPI_UPDATE_FEED_URL}")
endif()

if(WIN32)
    # A GUI app (no console window), with an icon and version information
    set(DSPI_WIN_DIR ${CMAKE_SOURCE_DIR}/../packaging/windows)
    configure_file(${DSPI_WIN_DIR}/dspi-console.rc.in ${CMAKE_CURRENT_BINARY_DIR}/dspi-console.rc @ONLY)
    target_sources(DSPiConsole PRIVATE ${CMAKE_CURRENT_BINARY_DIR}/dspi-console.rc)
    set_target_properties(DSPiConsole PROPERTIES WIN32_EXECUTABLE ON)
    # Windows 11 rounded corners for the frameless windows (WindowEffects)
    target_link_libraries(DSPiConsole PRIVATE dwmapi)
    # -- WP QD8 -- Link tokens in Credential Manager
    target_link_libraries(DSPiConsole PRIVATE advapi32)
    # -- WP QD18 -- The audio endpoints' format (MMDevice, COM)
    target_link_libraries(DSPiConsole PRIVATE ole32)

    # Windows: the core is a DLL (dspi_capi.dll) next to the exe, linked
    # through cargo's import library
    if(MSVC)
        target_link_libraries(DSPiConsole PRIVATE ${DSPI_CORE_LIBDIR}/${DSPI_CORE_LIB}.dll.lib)
    else()
        target_link_libraries(DSPiConsole PRIVATE ${DSPI_CORE_LIBDIR}/lib${DSPI_CORE_LIB}.dll.a)
    endif()
    add_custom_command(TARGET DSPiConsole POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different ${DSPI_CORE_LIBDIR}/${DSPI_CORE_LIB}.dll $<TARGET_FILE_DIR:DSPiConsole>)
else()
    target_link_libraries(DSPiConsole PRIVATE ${DSPI_CORE_LIBDIR}/lib${DSPI_CORE_LIB}.a)
endif()

if(APPLE)
    # An app bundle, "DSPi uConsole.app", with its icon and Info.plist
    set(DSPI_MAC_DIR ${CMAKE_SOURCE_DIR}/../packaging/macos)
    set_source_files_properties(${DSPI_MAC_DIR}/dspi-console.icns PROPERTIES MACOSX_PACKAGE_LOCATION Resources)
    target_sources(DSPiConsole PRIVATE ${DSPI_MAC_DIR}/dspi-console.icns)
    set_target_properties(DSPiConsole PROPERTIES
        MACOSX_BUNDLE ON
        OUTPUT_NAME "DSPi uConsole"
        MACOSX_BUNDLE_INFO_PLIST ${DSPI_MAC_DIR}/Info.plist.in
        # -- WP QD21b -- a neutral placeholder: .invalid is reserved (RFC 6761), so
        # no one can hold it; a real id needs a domain the maintainers own
        MACOSX_BUNDLE_GUI_IDENTIFIER invalid.dspi-unified-console.DSPiConsole
        MACOSX_BUNDLE_SHORT_VERSION_STRING ${DSPI_VER_MAJOR}.${DSPI_VER_MINOR}.${DSPI_VER_PATCH}
        MACOSX_BUNDLE_BUNDLE_VERSION ${DSPI_VERSION})

    # The native Console's system colours, read from AppKit
    enable_language(OBJCXX)
    target_sources(DSPiConsole PRIVATE src/MacSystemColors.mm)
    set_source_files_properties(src/MacSystemColors.mm PROPERTIES COMPILE_OPTIONS -fobjc-arc)
    target_link_libraries(DSPiConsole PRIVATE "-framework AppKit")

    # The macOS frameworks the core's USB stack (nusb) and Rust's std use
    find_library(IOKIT IOKit)
    find_library(CF CoreFoundation)
    find_library(SEC Security)
    find_library(OBJC objc)
    target_link_libraries(DSPiConsole PRIVATE ${IOKIT} ${CF} ${SEC} ${OBJC}
        "-framework SystemConfiguration")
    # -- WP QD8 -- The core's MIDI ports (midir)
    target_link_libraries(DSPiConsole PRIVATE "-framework CoreMIDI" "-framework CoreAudio")
elseif(UNIX)
    # Linux: the core talks to usbfs itself (nusb); it needs only these, and ALSA
    # for its MIDI ports (midir). -- WP QD8 --
    target_link_libraries(DSPiConsole PRIVATE pthread dl m asound Qt::DBus)

    # Optional: KDE blur-behind support
    # KF5 with Qt 5, KF6 with Qt 6
    find_package(KF${QT_VERSION_MAJOR}WindowSystem QUIET)
    if(KF${QT_VERSION_MAJOR}WindowSystem_FOUND)
        target_link_libraries(DSPiConsole PRIVATE KF${QT_VERSION_MAJOR}::WindowSystem)
        target_compile_definitions(DSPiConsole PRIVATE HAS_KDE_BLUR)
        message(STATUS "KDE blur-behind support enabled")
    else()
        message(STATUS "KDE Frameworks not found — blur-behind disabled")
    endif()
endif()

# ── Install and packages (Linux) ──
# `cmake --install` lays out a system install; `cpack -G DEB` / `cpack -G RPM`
# build the packages (packaging/linux/build-release.sh runs them).
if(UNIX AND NOT APPLE)
    # Packages sort a pre-release before its release with ~: 0.14.0~beta1 < 0.14.0
    string(REPLACE "-" "~" DSPI_PKG_VERSION "${DSPI_VERSION}")

    set(PKG ${CMAKE_SOURCE_DIR}/../packaging/linux)
    install(PROGRAMS $<TARGET_FILE:DSPiConsole> DESTINATION bin RENAME dspi-uconsole)
    install(FILES ${PKG}/dspi-uconsole.desktop DESTINATION share/applications)
    install(FILES ${CMAKE_SOURCE_DIR}/dspi-console.svg DESTINATION share/icons/hicolor/scalable/apps RENAME dspi-uconsole.svg)
    install(FILES ${PKG}/dspi-uconsole.metainfo.xml DESTINATION share/metainfo)
    # Named after the package, so it installs next to another package's 70-dspi.rules
    install(FILES ${PKG}/70-dspi.rules DESTINATION lib/udev/rules.d RENAME 70-dspi-uconsole.rules)
    # -- WP QD42 -- The licence and the notice (Qt is the distribution's): in
    # the .deb, Debian's copyright file, pointing at Debian's copy of the GPL;
    # elsewhere (the .rpm, Arch, a plain install) the GPL's text and the notice
    # in share/licenses, as Fedora and Arch expect
    set(DSPI_PACKAGE_FORMAT "" CACHE STRING "deb: the licence as Debian lays it out (build-debian.sh)")
    set(LIC ${CMAKE_SOURCE_DIR}/../packaging/licences)
    # -- WP LIC1 -- the additional terms (GPL-3.0 section 7) in the notice, verbatim,
    # and the Rust crates' notices next to it
    file(READ ${LIC}/TERMS.txt DSPI_TERMS)
    set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS ${LIC}/TERMS.txt)
    if(DSPI_PACKAGE_FORMAT STREQUAL "deb")
        set(DSPI_GPL_TEXT "/usr/share/common-licenses/GPL-3")
        set(DSPI_RUST_CRATES "/usr/share/doc/dspi-uconsole/rust-crates.txt")
        configure_file(${LIC}/NOTICE-distro.txt.in ${CMAKE_BINARY_DIR}/copyright @ONLY)
        install(FILES ${CMAKE_BINARY_DIR}/copyright ${LIC}/rust-crates.txt DESTINATION share/doc/dspi-uconsole)
    else()
        set(DSPI_GPL_TEXT "GPL-3.0.txt, next to this file,")
        set(DSPI_RUST_CRATES "rust-crates.txt, next to this file")
        configure_file(${LIC}/NOTICE-distro.txt.in ${CMAKE_BINARY_DIR}/NOTICE.txt @ONLY)
        install(FILES ${LIC}/GPL-3.0.txt ${CMAKE_BINARY_DIR}/NOTICE.txt ${LIC}/TERMS.txt ${LIC}/rust-crates.txt
            DESTINATION share/licenses/dspi-uconsole)
    endif()

    set(CPACK_PACKAGE_NAME dspi-uconsole)
    set(CPACK_PACKAGE_VERSION ${DSPI_PKG_VERSION})
    # Placeholder maker until the project has a home of its own: no address, no homepage
    set(CPACK_PACKAGE_VENDOR "DSPi unified console maintainers")
    set(CPACK_PACKAGE_CONTACT "DSPi unified console maintainers")
    set(CPACK_PACKAGE_DESCRIPTION_SUMMARY "Control application for the DSPi audio processor")
    set(CPACK_PACKAGE_DESCRIPTION "DSPi uConsole configures and tunes a DSPi, open source DSP firmware for the Raspberry Pi Pico: parametric EQ, crossovers, routing, delay, effects, limiters, control surfaces and firmware updates, applied live over USB.")
    set(CPACK_PACKAGING_INSTALL_PREFIX /usr)
    set(CPACK_STRIP_FILES ON)

    # Debian / Ubuntu: library dependencies from dpkg-shlibdeps, plus the QML
    # modules and plugins the binary loads at run time
    set(CPACK_DEBIAN_FILE_NAME DEB-DEFAULT)
    set(CPACK_DEBIAN_PACKAGE_SECTION sound)
    set(CPACK_DEBIAN_PACKAGE_SHLIBDEPS ON)
    if(QT_VERSION_MAJOR EQUAL 5)
        set(CPACK_DEBIAN_PACKAGE_DEPENDS "qml-module-qtquick2, qml-module-qtquick-window2, qml-module-qtquick-layouts, qml-module-qtquick-controls2, qml-module-qtquick-templates2, qml-module-qtqml, qml-module-qt-labs-settings, qml-module-qt-labs-platform, libqt5svg5 | libqt5svg5t64, qtwayland5")
    else()
        set(CPACK_DEBIAN_PACKAGE_DEPENDS "qml6-module-qtquick, qml6-module-qtquick-window, qml6-module-qtquick-layouts, qml6-module-qtquick-controls, qml6-module-qtquick-templates, qml6-module-qtqml, qml6-module-qtqml-workerscript, qml6-module-qt-labs-settings, qml6-module-qt-labs-platform, libqt6svg6, qt6-svg-plugins, qt6-wayland")
    endif()
    set(CPACK_DEBIAN_PACKAGE_RECOMMENDS "kwayland-integration, xdg-desktop-portal, udisks2")
    set(CPACK_DEBIAN_PACKAGE_CONTROL_EXTRA "${PKG}/postinst;${PKG}/postrm")

    # Fedora: library dependencies are found automatically
    set(CPACK_RPM_FILE_NAME RPM-DEFAULT)
    set(CPACK_RPM_PACKAGE_RELEASE 1)
    set(CPACK_RPM_PACKAGE_LICENSE "GPL-3.0-or-later")   # -- WP QD42 -- the owner's decision, 2026-11-08
    set(CPACK_RPM_PACKAGE_GROUP "Applications/Multimedia")
    if(QT_VERSION_MAJOR EQUAL 5)
        set(CPACK_RPM_PACKAGE_REQUIRES "qt5-qtdeclarative, qt5-qtquickcontrols2, qt5-qtsvg, qt5-qtwayland")
    else()
        set(CPACK_RPM_PACKAGE_REQUIRES "qt6-qtdeclarative, qt6-qtsvg, qt6-qtwayland")
    endif()
    set(CPACK_RPM_PACKAGE_SUGGESTS "kwayland-integration, xdg-desktop-portal, udisks2")
    set(CPACK_RPM_POST_INSTALL_SCRIPT_FILE ${PKG}/postinst)
    set(CPACK_RPM_POST_UNINSTALL_SCRIPT_FILE ${PKG}/postrm)
    # Don't claim ownership of system directories
    set(CPACK_RPM_EXCLUDE_FROM_AUTO_FILELIST_ADDITION
        /usr/share/applications /usr/share/icons /usr/share/icons/hicolor /usr/share/icons/hicolor/scalable
        /usr/share/icons/hicolor/scalable/apps /usr/share/metainfo /usr/lib/udev /usr/lib/udev/rules.d
        /usr/share/licenses)   # -- WP QD42 --

    include(CPack)
endif()
