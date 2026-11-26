# -- WP GAP2 -- "Solo this output": the core's listening state (dspi_solo_*),
# the channel menu's row, the Routing inspector's switch and the status strip's
# banner
list(APPEND DSPI_APP_SOURCES
    src/controllers/SoloController.h src/controllers/SoloController.cpp)
list(APPEND DSPI_QML_FILES
    qml/banners/SoloBanner.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeGap2.cpp)
