# -- WP LKUI1 -- The hub's audio link installation: Window > System... (units,
# alink state, scene bar, Clock and audio links) and a status line under the menu bar
list(APPEND DSPI_APP_SOURCES
    src/controllers/SystemController.h src/controllers/SystemController.cpp)
# -- WP INSTQ -- SystemWindow.qml is SystemPanel.qml (instq.cmake)
list(APPEND DSPI_QML_FILES qml/system/SystemStatusStrip.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeAlinkSystem.cpp)
