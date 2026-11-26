# -- WP UB15 -- (UXP B15) Claude Desktop in one click: the ClaudeDesktop singleton
# (the extension Settings › AI Agents installs, where Claude Desktop's
# configuration is), the app's icon for the extension at :/ub15/dspi-icon-rounded-1024.png, and
# the smoke sections ub15-desktop (checks) and ub15-pictures (the report's)
list(APPEND DSPI_APP_SOURCES
    src/controllers/ClaudeDesktopController.h src/controllers/ClaudeDesktopController.cpp)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeUb15.cpp)
function(dspi_ub15_target target)
    qt_add_resources(${target} dspi_ub15_icon
        PREFIX "/ub15"
        BASE "${DSPI_REPO}/assets/icon"
        FILES "${DSPI_REPO}/assets/icon/dspi-icon-rounded-1024.png")
endfunction()
list(APPEND DSPI_APP_HOOKS dspi_ub15_target)
