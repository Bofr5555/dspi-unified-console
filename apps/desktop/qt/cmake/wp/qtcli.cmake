# -- WP QTCLI -- The device-access dialog's udev rule is the packages' own
# (packaging/linux/70-uconsole-for-dspi.rules), a resource at
# :/udev/70-uconsole-for-dspi.rules on each target, so the two cannot drift.
# The smoke driver compares it with the source file (DSPI_UDEV_RULE_SOURCE).
set(DSPI_UDEV_RULE "${CMAKE_CURRENT_SOURCE_DIR}/../packaging/linux/70-uconsole-for-dspi.rules")
function(dspi_qtcli_target target)
    qt_add_resources(${target} dspi_qtcli_udev
        PREFIX "/udev"
        BASE "${CMAKE_CURRENT_SOURCE_DIR}/../packaging/linux"
        FILES "${DSPI_UDEV_RULE}")
    target_compile_definitions(${target} PRIVATE DSPI_UDEV_RULE_SOURCE="${DSPI_UDEV_RULE}")
endfunction()
list(APPEND DSPI_APP_HOOKS dspi_qtcli_target)
