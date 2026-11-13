# -- WP LKUI1 -- The hub's audio link installation: Window > System... (units,
# link state, scene bar, Clock and links) and a status line under the menu bar
list(APPEND DSPI_APP_SOURCES
    src/controllers/SystemController.h src/controllers/SystemController.cpp)
list(APPEND DSPI_QML_FILES qml/system/SystemWindow.qml qml/system/SystemStatusStrip.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLinkSystem.cpp)
