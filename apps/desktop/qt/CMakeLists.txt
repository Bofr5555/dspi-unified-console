cmake_minimum_required(VERSION 3.16)
project(DSPiConsole VERSION 1.0 LANGUAGES CXX)
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
    find_package(Qt6 6.2 REQUIRED COMPONENTS Core Gui Widgets Quick QuickControls2 Qml Network)
endif()
message(STATUS "Building against Qt ${QT_VERSION}")

# The Rust core (dspi-core). Build it first with cargo; these say where its
# header and library are. DSPI_RUST_TARGET is the cargo --target triple when
# cross-compiling (e.g. x86_64-pc-windows-gnu), empty for a native build.
set(DSPI_CORE_DIR "${CMAKE_SOURCE_DIR}/.." CACHE PATH "Directory holding include/dspi_core.h")
set(DSPI_CARGO_TARGET_DIR "${DSPI_CORE_DIR}/target" CACHE PATH "cargo's target directory")
set(DSPI_RUST_TARGET "" CACHE STRING "cargo --target triple, empty for the host")
if(DSPI_RUST_TARGET)
    set(DSPI_CORE_LIBDIR "${DSPI_CARGO_TARGET_DIR}/${DSPI_RUST_TARGET}/release")
else()
    set(DSPI_CORE_LIBDIR "${DSPI_CARGO_TARGET_DIR}/release")
endif()

# The version the app reports, e.g. 1.1.6-beta4
file(STRINGS ${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp _ver REGEX "setApplicationVersion\\(\"[^\"]+\"\\)")
string(REGEX REPLACE ".*setApplicationVersion\\(\"([^\"]+)\"\\).*" "\\1" DSPI_VERSION "${_ver}")
string(REGEX MATCH "^([0-9]+)\\.([0-9]+)\\.([0-9]+)" _ "${DSPI_VERSION}")
set(DSPI_VER_MAJOR ${CMAKE_MATCH_1})
set(DSPI_VER_MINOR ${CMAKE_MATCH_2})
set(DSPI_VER_PATCH ${CMAKE_MATCH_3})

add_executable(DSPiConsole
    src/main.cpp
    src/DSPiBridge.cpp
    src/BodePlotItem.cpp
    src/PeqEditorItem.cpp
    src/RtaController.cpp
    src/RtaViews.cpp
    src/PointerTracker.cpp
    src/SiggenController.cpp
    src/CsController.cpp
    src/FirmwareUpdater.cpp
    src/AutoEqLibrary.cpp
    src/StatsController.cpp
    src/MonitorModel.cpp
    src/ConfigFiles.cpp
    src/MeterItem.cpp
    src/WindowEffects.cpp
    resources.qrc
)

target_include_directories(DSPiConsole PRIVATE
    ${DSPI_CORE_DIR}/include
    src/
)

target_link_libraries(DSPiConsole PRIVATE
    Qt::Core Qt::Gui Qt::Widgets Qt::Quick Qt::QuickControls2 Qt::Qml Qt::Network
)

if(WIN32)
    # A GUI app (no console window), with an icon and version information
    set(DSPI_WIN_DIR ${CMAKE_SOURCE_DIR}/../packaging/windows)
    configure_file(${DSPI_WIN_DIR}/dspi-console.rc.in ${CMAKE_CURRENT_BINARY_DIR}/dspi-console.rc @ONLY)
    target_sources(DSPiConsole PRIVATE ${CMAKE_CURRENT_BINARY_DIR}/dspi-console.rc)
    set_target_properties(DSPiConsole PROPERTIES WIN32_EXECUTABLE ON)
    # Windows 11 rounded corners for the frameless windows (WindowEffects)
    target_link_libraries(DSPiConsole PRIVATE dwmapi)

    # Windows: the core is a DLL (dspi_core.dll) next to the exe, linked
    # through cargo's import library
    if(MSVC)
        target_link_libraries(DSPiConsole PRIVATE ${DSPI_CORE_LIBDIR}/dspi_core.dll.lib)
    else()
        target_link_libraries(DSPiConsole PRIVATE ${DSPI_CORE_LIBDIR}/libdspi_core.dll.a)
    endif()
    add_custom_command(TARGET DSPiConsole POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different ${DSPI_CORE_LIBDIR}/dspi_core.dll $<TARGET_FILE_DIR:DSPiConsole>)
else()
    target_link_libraries(DSPiConsole PRIVATE ${DSPI_CORE_LIBDIR}/libdspi_core.a)
endif()

if(APPLE)
    # The native Console's system colours, read from AppKit
    enable_language(OBJCXX)
    target_sources(DSPiConsole PRIVATE src/MacSystemColors.mm)
    set_source_files_properties(src/MacSystemColors.mm PROPERTIES COMPILE_OPTIONS -fobjc-arc)
    target_link_libraries(DSPiConsole PRIVATE "-framework AppKit")

    # libusb from Homebrew
    find_library(LIBUSB usb-1.0 HINTS /opt/homebrew/opt/libusb/lib)
    target_include_directories(DSPiConsole PRIVATE /opt/homebrew/opt/libusb/include)
    target_link_libraries(DSPiConsole PRIVATE ${LIBUSB})

    # macOS frameworks required by rusb/libusb
    find_library(IOKIT IOKit)
    find_library(CF CoreFoundation)
    find_library(SEC Security)
    find_library(OBJC objc)
    target_link_libraries(DSPiConsole PRIVATE ${IOKIT} ${CF} ${SEC} ${OBJC}
        "-framework SystemConfiguration")
elseif(UNIX)
    # Linux: libusb via pkg-config, plus pthreads and dl
    find_package(PkgConfig REQUIRED)
    pkg_check_modules(LIBUSB REQUIRED libusb-1.0)
    target_include_directories(DSPiConsole PRIVATE ${LIBUSB_INCLUDE_DIRS})
    target_link_libraries(DSPiConsole PRIVATE ${LIBUSB_LIBRARIES} pthread dl udev)

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
    # Packages sort a pre-release before its release with ~: 1.1.6~beta4 < 1.1.6
    string(REPLACE "-" "~" DSPI_PKG_VERSION "${DSPI_VERSION}")

    set(PKG ${CMAKE_SOURCE_DIR}/../packaging/linux)
    install(PROGRAMS $<TARGET_FILE:DSPiConsole> DESTINATION bin RENAME dspi-console)
    install(FILES ${PKG}/dspi-console.desktop DESTINATION share/applications)
    install(FILES ${CMAKE_SOURCE_DIR}/dspi-console.svg DESTINATION share/icons/hicolor/scalable/apps)
    install(FILES ${PKG}/io.github.weeblabs.DSPiConsole.metainfo.xml DESTINATION share/metainfo)
    install(FILES ${PKG}/70-dspi.rules DESTINATION lib/udev/rules.d)

    set(CPACK_PACKAGE_NAME dspi-console)
    set(CPACK_PACKAGE_VERSION ${DSPI_PKG_VERSION})
    set(CPACK_PACKAGE_VENDOR "WeebLabs")
    set(CPACK_PACKAGE_CONTACT "WeebLabs <https://github.com/WeebLabs/DSPi-Console-Linux>")
    set(CPACK_PACKAGE_DESCRIPTION_SUMMARY "Control application for the DSPi audio processor")
    set(CPACK_PACKAGE_DESCRIPTION "DSPi Console configures and tunes a DSPi, open source DSP firmware for the Raspberry Pi Pico: parametric EQ, crossovers, routing, delay, effects, limiters, control surfaces and firmware updates, applied live over USB.")
    set(CPACK_PACKAGE_HOMEPAGE_URL "https://github.com/WeebLabs/DSPi-Console-Linux")
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
        set(CPACK_DEBIAN_PACKAGE_DEPENDS "qml6-module-qtquick, qml6-module-qtquick-window, qml6-module-qtquick-layouts, qml6-module-qtquick-controls, qml6-module-qtquick-templates, qml6-module-qtqml, qml6-module-qtqml-workerscript, qml6-module-qt-labs-settings, qml6-module-qt-labs-platform, libqt6svg6, qt6-wayland")
    endif()
    set(CPACK_DEBIAN_PACKAGE_RECOMMENDS "kwayland-integration, xdg-desktop-portal, udisks2")
    set(CPACK_DEBIAN_PACKAGE_CONTROL_EXTRA "${PKG}/postinst;${PKG}/postrm")

    # Fedora: library dependencies are found automatically
    set(CPACK_RPM_FILE_NAME RPM-DEFAULT)
    set(CPACK_RPM_PACKAGE_RELEASE 1)
    set(CPACK_RPM_PACKAGE_LICENSE "Unspecified")
    set(CPACK_RPM_PACKAGE_GROUP "Applications/Multimedia")
    set(CPACK_RPM_PACKAGE_URL "https://github.com/WeebLabs/DSPi-Console-Linux")
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
        /usr/share/icons/hicolor/scalable/apps /usr/share/metainfo /usr/lib/udev /usr/lib/udev/rules.d)

    include(CPack)
endif()
