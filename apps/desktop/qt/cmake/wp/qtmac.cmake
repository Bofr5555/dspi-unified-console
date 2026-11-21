# -- WP QTMAC -- macOS: the app bundle "uConsole for DSPi.app" (its Info.plist
# from packaging/macos/Info.plist.in, its icon made from assets/icon, its
# version), and the system frameworks the core's static library links. The
# .dmg is made by packaging/macos/build-macos.sh. Every line is inside
# if(APPLE): the Linux and Windows builds do not change.
if(APPLE)
    set(DSPI_MACOS_BUNDLE_ID "se.bofr.uconsole-for-dspi" CACHE STRING "The macOS app's bundle identifier")

    # What cargo's native-static-libs names for dspi-capi on macOS (both
    # architectures: -framework IOKit -framework CoreAudio -lobjc -framework
    # Foundation -framework CoreMIDI -framework CoreFoundation -liconv; libSystem
    # comes with every link)
    set_property(TARGET dspi_capi APPEND PROPERTY INTERFACE_LINK_LIBRARIES
        "-framework IOKit" "-framework CoreAudio" objc "-framework Foundation" "-framework CoreMIDI"
        "-framework CoreFoundation" iconv)

    # Qt 6.8.3's FindWrapOpenGL links "-framework AGL", which the macOS 26 SDK
    # (Xcode 26) no longer has, so the link fails (later Qt links it only if
    # found). The app does not use AGL; QtGui, which does, loads it from the
    # system, which still has it.
    if(TARGET WrapOpenGL::WrapOpenGL)
        get_target_property(qtmac_gl WrapOpenGL::WrapOpenGL INTERFACE_LINK_LIBRARIES)
        if(qtmac_gl)
            list(FILTER qtmac_gl EXCLUDE REGEX "AGL")
            set_property(TARGET WrapOpenGL::WrapOpenGL PROPERTY INTERFACE_LINK_LIBRARIES "${qtmac_gl}")
        endif()
    endif()

    # The icon, from the rounded 1024 px PNG every client's icon comes from:
    # sips scales it, iconutil packs the sizes macOS asks for
    set(DSPI_QTMAC_PNG "${DSPI_REPO}/assets/icon/dspi-icon-rounded-1024.png")
    set(DSPI_QTMAC_ICONSET "${CMAKE_CURRENT_BINARY_DIR}/qtmac/uconsole-for-dspi.iconset")
    set(DSPI_QTMAC_ICNS "${CMAKE_CURRENT_BINARY_DIR}/qtmac/uconsole-for-dspi.icns")
    set(qtmac_sizes)
    foreach(size 16 32 128 256 512)
        math(EXPR double "${size} * 2")
        list(APPEND qtmac_sizes
            COMMAND sips -z ${size} ${size} "${DSPI_QTMAC_PNG}" --out "${DSPI_QTMAC_ICONSET}/icon_${size}x${size}.png"
            COMMAND sips -z ${double} ${double} "${DSPI_QTMAC_PNG}" --out "${DSPI_QTMAC_ICONSET}/icon_${size}x${size}@2x.png")
    endforeach()
    add_custom_command(OUTPUT "${DSPI_QTMAC_ICNS}"
        COMMAND ${CMAKE_COMMAND} -E rm -rf "${DSPI_QTMAC_ICONSET}"
        COMMAND ${CMAKE_COMMAND} -E make_directory "${DSPI_QTMAC_ICONSET}"
        ${qtmac_sizes}
        COMMAND iconutil -c icns "${DSPI_QTMAC_ICONSET}" -o "${DSPI_QTMAC_ICNS}"
        DEPENDS "${DSPI_QTMAC_PNG}"
        COMMENT "Making the app icon (uconsole-for-dspi.icns)"
        VERBATIM)
    set_source_files_properties("${DSPI_QTMAC_ICNS}" PROPERTIES MACOSX_PACKAGE_LOCATION Resources GENERATED TRUE)

    # The app only; the smoke build stays a plain executable
    function(dspi_qtmac_target target)
        if(NOT target STREQUAL "uconsole-for-dspi")
            return()
        endif()
        target_sources(${target} PRIVATE "${DSPI_QTMAC_ICNS}")
        set_target_properties(${target} PROPERTIES
            MACOSX_BUNDLE TRUE
            OUTPUT_NAME "${DSPI_DISPLAY_NAME}"
            MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/../packaging/macos/Info.plist.in"
            MACOSX_BUNDLE_GUI_IDENTIFIER "${DSPI_MACOS_BUNDLE_ID}"
            MACOSX_BUNDLE_BUNDLE_NAME "${DSPI_DISPLAY_NAME}"
            MACOSX_BUNDLE_ICON_FILE uconsole-for-dspi
            MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
            MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}"
            MACOSX_BUNDLE_COPYRIGHT "DSPi unified console maintainers")
    endfunction()
    list(APPEND DSPI_APP_HOOKS dspi_qtmac_target)

    # The smoke build (DSPI_SMOKE, a test target that is never shipped) on a Mac:
    # without the two sections that drive Linux-only code through QtDBus (the
    # Secret Service token store, qp13-link; the XDG login item and the logind
    # inhibitor, qp14-system). The others run as on Linux.
    list(REMOVE_ITEM DSPI_SMOKE_SOURCES smoke/SmokeLink.cpp smoke/SmokeSystem.cpp)
endif()
