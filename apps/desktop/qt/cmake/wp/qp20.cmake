# -- WP QP20 -- Packaging: the Windows exe's resources, and on Linux what a
# package installs besides the base's files (the udev rule, the notices) and
# the .deb / .rpm themselves (CPack). packaging/README.md describes the
# packages; nothing here changes the app.
set(DSPI_PACKAGING "${CMAKE_CURRENT_SOURCE_DIR}/../packaging")

# The name people see, from AppIdentity (one place for it)
file(STRINGS src/AppIdentity.h DSPI_QP20_NAME_LINE REGEX "displayName = \"")
string(REGEX REPLACE ".*displayName = \"([^\"]*)\".*" "\\1" DSPI_DISPLAY_NAME "${DSPI_QP20_NAME_LINE}")

# The release's source archive the notices name (ci/release-source.sh --with-desktop)
set(DSPI_SOURCE_ASSET "dspi-uconsole-${PROJECT_VERSION}-desktop-source.tar.gz")

# ---- Windows: the exe's icon and version information --------------------------
if(WIN32)
    # cargo names the MinGW import library libdspi_capi.dll.a; the base's
    # import looks for dspi_capi.dll.a (the name older toolchains wrote)
    if(NOT EXISTS "${DSPI_CAPI_DIR}/dspi_capi.dll.a")
        set_target_properties(dspi_capi PROPERTIES IMPORTED_LOCATION "${DSPI_CAPI_DIR}/libdspi_capi.dll.a")
    endif()
    set(DSPI_WIN_DIR "${DSPI_PACKAGING}/windows")
    set(DSPI_VERSION "${PROJECT_VERSION}")
    set(DSPI_VER_MAJOR "${PROJECT_VERSION_MAJOR}")
    set(DSPI_VER_MINOR "${PROJECT_VERSION_MINOR}")
    set(DSPI_VER_PATCH "${PROJECT_VERSION_PATCH}")
    configure_file("${DSPI_WIN_DIR}/dspi-console.rc.in" "${CMAKE_CURRENT_BINARY_DIR}/dspi-console.rc" @ONLY)
    list(APPEND DSPI_APP_SOURCES "${CMAKE_CURRENT_BINARY_DIR}/dspi-console.rc")
endif()

# ---- Linux: the udev rule and the notices --------------------------------------
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    include(GNUInstallDirs)
    # Where udev reads a package's rules; empty installs none (the AppImage,
    # whose archive carries the rule for install.sh)
    set(DSPI_UDEV_RULES_DIR "lib/udev/rules.d" CACHE STRING
        "udev rules directory under the prefix; empty: no rule")
    if(DSPI_UDEV_RULES_DIR)
        install(FILES "${DSPI_PACKAGING}/linux/70-dspi-uconsole.rules" DESTINATION "${DSPI_UDEV_RULES_DIR}")
    endif()

    # The licence files, laid out as the AppImage and Windows stage them
    # (NOTICE.txt and LICENSES/, which About lists), where the distribution
    # wants them:
    #   rpm   share/licenses/dspi-uconsole/ (also a plain cmake --install)
    #   deb   share/doc/dspi-uconsole/, with Debian's copyright file; the GPL
    #         is the system's /usr/share/common-licenses/GPL-3
    #   none  nothing (the AppImage: packaging/licences/stage.sh)
    set(DSPI_NOTICES "rpm" CACHE STRING "Licence files to install: rpm, deb or none")
    set_property(CACHE DSPI_NOTICES PROPERTY STRINGS rpm deb none)
    if(DSPI_NOTICES STREQUAL "rpm")
        set(DSPI_NOTICES_DIR "${CMAKE_INSTALL_DATADIR}/licenses/dspi-uconsole")
        set(DSPI_GPL_TEXT "LICENSES/GPL-3.0.txt")
    elseif(DSPI_NOTICES STREQUAL "deb")
        set(DSPI_NOTICES_DIR "${CMAKE_INSTALL_DATADIR}/doc/dspi-uconsole")
        set(DSPI_GPL_TEXT "/usr/share/common-licenses/GPL-3")
    elseif(NOT DSPI_NOTICES STREQUAL "none")
        message(FATAL_ERROR "DSPI_NOTICES=${DSPI_NOTICES}: rpm, deb or none")
    endif()
    if(DEFINED DSPI_NOTICES_DIR)
        set(DSPI_VERSION "${PROJECT_VERSION}")
        file(READ "${DSPI_PACKAGING}/licences/TERMS.txt" DSPI_TERMS)
        configure_file("${DSPI_PACKAGING}/licences/NOTICE-distro.txt.in"
                       "${CMAKE_CURRENT_BINARY_DIR}/packaging/NOTICE.txt" @ONLY)
        install(FILES "${CMAKE_CURRENT_BINARY_DIR}/packaging/NOTICE.txt" DESTINATION "${DSPI_NOTICES_DIR}")
        if(DSPI_NOTICES STREQUAL "deb")
            install(FILES "${CMAKE_CURRENT_BINARY_DIR}/packaging/NOTICE.txt" DESTINATION "${DSPI_NOTICES_DIR}"
                    RENAME copyright)
        else()
            install(FILES "${DSPI_PACKAGING}/licences/GPL-3.0.txt" DESTINATION "${DSPI_NOTICES_DIR}/LICENSES")
        endif()
        install(FILES "${DSPI_PACKAGING}/licences/TERMS.txt" "${DSPI_PACKAGING}/licences/rust-crates.txt"
                      "${DSPI_REPO}/assets/licences/firmware-THIRD-PARTY-NOTICES.txt"
                      "${DSPI_REPO}/assets/licences/AutoEq-MIT.txt"
                DESTINATION "${DSPI_NOTICES_DIR}/LICENSES")
        install(FILES icons/lucide/LICENSE DESTINATION "${DSPI_NOTICES_DIR}/LICENSES/Lucide")
    endif()

    # ---- The .deb and the .rpm (cpack -G DEB / RPM, in packaging/linux's images)
    set(CPACK_PACKAGE_NAME dspi-uconsole)
    set(CPACK_PACKAGE_VERSION "${PROJECT_VERSION}")
    set(CPACK_PACKAGE_VENDOR "DSPi unified console maintainers")
    set(CPACK_PACKAGE_DESCRIPTION_SUMMARY "${DSPI_DISPLAY_NAME}: set up and control a DSPi audio processor")
    set(CPACK_PACKAGE_DESCRIPTION
        "DSPi turns a Raspberry Pi Pico or Pico 2 into a USB sound card with a digital signal processor inside it. This app shapes that processing: EQ, routing, delays, crossovers and effects, presets on the device, and sharing devices on the local network.")
    set(CPACK_PACKAGE_HOMEPAGE_URL "https://github.com/Bofr5555/dspi-unified-console")
    set(CPACK_PACKAGE_CONTACT "DSPi unified console maintainers <https://github.com/Bofr5555/dspi-unified-console/issues>")
    set(CPACK_PACKAGING_INSTALL_PREFIX /usr)
    set(CPACK_STRIP_FILES ON)
    set(CPACK_PACKAGE_FILE_NAME "dspi-uconsole-${PROJECT_VERSION}")

    # Debian: libraries from dpkg-shlibdeps; the QML modules and Qt's plugins
    # the app loads at run time are listed by hand
    set(CPACK_DEBIAN_FILE_NAME DEB-DEFAULT)
    set(CPACK_DEBIAN_PACKAGE_SECTION sound)
    set(CPACK_DEBIAN_PACKAGE_SHLIBDEPS ON)
    set(CPACK_DEBIAN_PACKAGE_DEPENDS
        "qml6-module-qtquick, qml6-module-qtquick-controls, qml6-module-qtquick-templates, qml6-module-qtquick-layouts, qml6-module-qtquick-window, qml6-module-qtqml, qml6-module-qtqml-workerscript, qml6-module-qtcore, qt6-qpa-plugins, qt6-svg-plugins")
    set(CPACK_DEBIAN_PACKAGE_RECOMMENDS "qt6-wayland, pkexec")
    set(CPACK_DEBIAN_PACKAGE_CONTROL_EXTRA "${DSPI_PACKAGING}/linux/postinst;${DSPI_PACKAGING}/linux/postrm")
    set(CPACK_DEBIAN_PACKAGE_CONTROL_STRICT_PERMISSION ON)

    # Fedora: libraries from rpmbuild's dependency generator; the QML modules by package
    set(CPACK_RPM_FILE_NAME RPM-DEFAULT)
    set(CPACK_RPM_PACKAGE_LICENSE "GPL-3.0-only")
    set(CPACK_RPM_PACKAGE_RELEASE 1)
    set(CPACK_RPM_PACKAGE_RELEASE_DIST ON)
    set(CPACK_RPM_PACKAGE_URL "${CPACK_PACKAGE_HOMEPAGE_URL}")
    set(CPACK_RPM_PACKAGE_REQUIRES "qt6-qtdeclarative >= 6.8, qt6-qtsvg >= 6.8")
    set(CPACK_RPM_PACKAGE_SUGGESTS "qt6-qtwayland, polkit")
    set(CPACK_RPM_POST_INSTALL_SCRIPT_FILE "${DSPI_PACKAGING}/linux/postinst")
    set(CPACK_RPM_POST_UNINSTALL_SCRIPT_FILE "${DSPI_PACKAGING}/linux/postrm")
    # Directories the system owns, not this package
    list(APPEND CPACK_RPM_EXCLUDE_FROM_AUTO_FILELIST_ADDITION
        /usr/lib/udev /usr/lib/udev/rules.d /usr/share/applications /usr/share/metainfo
        /usr/share/icons /usr/share/icons/hicolor /usr/share/icons/hicolor/scalable
        /usr/share/icons/hicolor/scalable/apps /usr/share/licenses /usr/share/doc)
    include(CPack)
endif()

# ---- Smoke: what About shows of the packages' notices ----------------------------
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokePackaging.cpp)
