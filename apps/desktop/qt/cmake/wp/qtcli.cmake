# -- WP QTCLI -- The device-access dialog's udev rule is the packages' own,
# a resource at :/udev/70-dspi.rules on each target, so the two cannot drift.
# -- WP UDEV1 -- It is the one rule file of every component (xtask/70-dspi.rules,
# also `uconsole --install-udev`'s). The smoke driver compares it with the
# source file (DSPI_UDEV_RULE_SOURCE).
get_filename_component(DSPI_UDEV_RULE "${CMAKE_CURRENT_SOURCE_DIR}/../../../xtask/70-dspi.rules" ABSOLUTE)
function(dspi_qtcli_target target)
    qt_add_resources(${target} dspi_qtcli_udev
        PREFIX "/udev"
        BASE "${CMAKE_CURRENT_SOURCE_DIR}/../../../xtask"
        FILES "${DSPI_UDEV_RULE}")
    target_compile_definitions(${target} PRIVATE DSPI_UDEV_RULE_SOURCE="${DSPI_UDEV_RULE}")
endfunction()
list(APPEND DSPI_APP_HOOKS dspi_qtcli_target)
