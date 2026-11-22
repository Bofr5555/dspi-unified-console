# -- WP LAY6 -- The command palette (Ctrl+K)
list(APPEND DSPI_APP_SOURCES
    src/controllers/PaletteController.h src/controllers/PaletteController.cpp)
list(APPEND DSPI_QML_FILES qml/modern/CommandPalette.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay6.cpp)
