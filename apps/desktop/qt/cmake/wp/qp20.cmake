# -- WP QP20 -- Packaging: the Windows exe's resources, and on Linux what a
# package installs besides the base's files (the udev rule, the notices) and
# the .deb / .rpm themselves (CPack). packaging/README.md describes the
# packages; nothing here changes the app.
set(DSPI_PACKAGING "${CMAKE_CURRENT_SOURCE_DIR}/../packaging")

# The name people see, from AppIdentity (one place for it)
file(STRINGS src/AppIdentity.h DSPI_QP20_NAME_LINE REGEX "displayName = \"")
string(REGEX REPLACE ".*displayName = \"([^\"]*)\".*" "\\1" DSPI_DISPLAY_NAME "${DSPI_QP20_NAME_LINE}")
if(NOT DSPI_DISPLAY_NAME)   # -- WP QPI4 -- never an empty name in a package
    message(FATAL_ERROR "no displayName = \"...\" line in src/AppIdentity.h")
endif()

# The release's source archive the notices name (ci/release-source.sh --with-desktop)
set(DSPI_SOURCE_ASSET "uConsole-for-DSPi-${PROJECT_VERSION}-desktop-source.tar.gz")

# ---- Windows: the exe's icon and version information --------------------------
if(WIN32)
    set(DSPI_WIN_DIR "${DSPI_PACKAGING}/windows")
    set(DSPI_VERSION "${PROJECT_VERSION}")
    set(DSPI_VER_MAJOR "${PROJECT_VERSION_MAJOR}")
    set(DSPI_VER_MINOR "${PROJECT_VERSION_MINOR}")
    set(DSPI_VER_PATCH "${PROJECT_VERSION_PATCH}")
    configure_file("${DSPI_WIN_DIR}/uconsole-for-dspi.rc.in" "${CMAKE_CURRENT_BINARY_DIR}/uconsole-for-dspi.rc" @ONLY)
    list(APPEND DSPI_APP_SOURCES "${CMAKE_CURRENT_BINARY_DIR}/uconsole-for-dspi.rc")
endif()

# ---- Linux: the udev rule and the notices --------------------------------------
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    include(GNUInstallDirs)
    # Where udev reads a package's rules; empty installs none (the AppImage,
    # whose archive carries the rule for install.sh)
    set(DSPI_UDEV_RULES_DIR "lib/udev/rules.d" CACHE STRING
        "udev rules directory under the prefix; empty: no rule")
    if(DSPI_UDEV_RULES_DIR)
        install(FILES "${DSPI_PACKAGING}/linux/70-uconsole-for-dspi.rules" DESTINATION "${DSPI_UDEV_RULES_DIR}")
    endif()

    # The licence files, laid out as the AppImage and Windows stage them
    # (NOTICE.txt and LICENSES/, which About lists), where the distribution
    # wants them:
    #   rpm   share/licenses/uconsole-for-dspi/ (also a plain cmake --install)
    #   deb   share/doc/uconsole-for-dspi/, with Debian's copyright file; the GPL
    #         is the system's /usr/share/common-licenses/GPL-3
    #   none  nothing (the AppImage: packaging/licences/stage.sh)
    set(DSPI_NOTICES "rpm" CACHE STRING "Licence files to install: rpm, deb or none")
    set_property(CACHE DSPI_NOTICES PROPERTY STRINGS rpm deb none)
    if(DSPI_NOTICES STREQUAL "rpm")
        set(DSPI_NOTICES_DIR "${CMAKE_INSTALL_DATADIR}/licenses/uconsole-for-dspi")
        set(DSPI_GPL_TEXT "LICENSES/GPL-3.0.txt")
    elseif(DSPI_NOTICES STREQUAL "deb")
        set(DSPI_NOTICES_DIR "${CMAKE_INSTALL_DATADIR}/doc/uconsole-for-dspi")
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
    set(CPACK_PACKAGE_NAME uconsole-for-dspi)
    set(CPACK_PACKAGE_VERSION "${PROJECT_VERSION}")
    set(CPACK_PACKAGE_VENDOR "DSPi unified console maintainers")
    set(CPACK_PACKAGE_DESCRIPTION_SUMMARY "${DSPI_DISPLAY_NAME}: set up and control a DSPi audio processor")
    set(CPACK_PACKAGE_DESCRIPTION
        "DSPi turns a Raspberry Pi Pico or Pico 2 into a USB sound card with a digital signal processor inside it. This app shapes that processing: EQ, routing, delays, crossovers and effects, presets on the device, and sharing devices on the local network.")
    set(CPACK_PACKAGE_HOMEPAGE_URL "https://github.com/Bofr5555/dspi-unified-console")
    set(CPACK_PACKAGE_CONTACT "DSPi unified console maintainers <https://github.com/Bofr5555/dspi-unified-console/issues>")
    # The package installs where the build was configured to (the scripts pass /usr)
    set(CPACK_PACKAGING_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}")
    set(CPACK_STRIP_FILES ON)
    set(CPACK_PACKAGE_FILE_NAME "uconsole-for-dspi-${PROJECT_VERSION}")
    # The packaging's own revision of a version, the same for both formats
    set(DSPI_PACKAGE_RELEASE 1)

    # Debian: the QML modules and Qt's plugins the app loads at run time come
    # from packaging/qt-modules.txt; the libraries it links from dpkg-shlibdeps
    set(CPACK_DEBIAN_FILE_NAME DEB-DEFAULT)   # uconsole-for-dspi_<version>_amd64.deb, the release's name
    set(CPACK_DEBIAN_PACKAGE_SECTION sound)   # Debian's section for audio applications
    set(CPACK_DEBIAN_PACKAGE_SHLIBDEPS ON)    # needs dpkg-dev (Containerfile.debian)
    # -- WP QPI4 -- from packaging/qt-modules.txt, which must name every
    # module qml/ imports
    file(STRINGS "${DSPI_PACKAGING}/qt-modules.txt" qpi4_rows REGEX "^(import|package) ")
    set(qpi4_listed "")
    set(qpi4_debs "")
    foreach(row IN LISTS qpi4_rows)
        string(REPLACE " " ";" row "${row}")
        list(GET row -1 deb)
        list(APPEND qpi4_debs "${deb}")
        list(GET row 0 kind)
        if(kind STREQUAL "import")
            list(GET row 1 module)
            list(APPEND qpi4_listed "${module}")
        endif()
    endforeach()
    file(GLOB_RECURSE qpi4_qml "${CMAKE_CURRENT_SOURCE_DIR}/qml/*.qml")
    foreach(f IN LISTS qpi4_qml)
        file(STRINGS "${f}" imports REGEX "^import Qt[A-Za-z.]+")
        foreach(line IN LISTS imports)
            string(REGEX MATCH "^import (Qt[A-Za-z.]+)" _ "${line}")
            if(NOT CMAKE_MATCH_1 IN_LIST qpi4_listed)
                message(FATAL_ERROR "${f} imports ${CMAKE_MATCH_1}, which packaging/qt-modules.txt does not list")
            endif()
        endforeach()
    endforeach()
    list(REMOVE_DUPLICATES qpi4_debs)
    list(JOIN qpi4_debs ", " CPACK_DEBIAN_PACKAGE_DEPENDS)
    set(CPACK_DEBIAN_PACKAGE_RECOMMENDS "qt6-wayland, pkexec")
    set(CPACK_DEBIAN_PACKAGE_CONTROL_EXTRA "${DSPI_PACKAGING}/linux/postinst;${DSPI_PACKAGING}/linux/postrm")
    set(CPACK_DEBIAN_PACKAGE_CONTROL_STRICT_PERMISSION ON)

    # Fedora: libraries from rpmbuild's dependency generator; the QML modules by package
    set(CPACK_RPM_FILE_NAME RPM-DEFAULT)   # uconsole-for-dspi-<version>-<release>.fc<N>.x86_64.rpm
    set(CPACK_RPM_PACKAGE_LICENSE "GPL-3.0-only")
    set(CPACK_RPM_PACKAGE_RELEASE ${DSPI_PACKAGE_RELEASE})
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
