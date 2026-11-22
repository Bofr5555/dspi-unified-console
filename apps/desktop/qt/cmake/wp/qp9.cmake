# -- WP QP9 -- Analysis: signal generator, spectrum analyser (window, graph
# overlay, bar strip), Stats for nerbs, Interrupt Monitor
list(APPEND DSPI_APP_SOURCES
    src/controllers/SiggenController.h src/controllers/SiggenController.cpp
    src/controllers/MonitorController.h src/controllers/MonitorController.cpp)
list(APPEND DSPI_QML_FILES
    qml/tools/SiggenPanel.qml qml/tools/SiggenTile.qml qml/tools/SiggenOutputChip.qml   # -- WP LAY1b -- was SiggenWindow.qml
    qml/tools/SiggenGlyphs.qml)   # -- WP LAY0 -- MonitorWindow.qml is MonitorPanel.qml (lay0.cmake)
list(APPEND DSPI_QML_SINGLETONS qml/tools/SiggenGlyphs.qml)
list(APPEND DSPI_APP_SOURCES
    src/controllers/StatsController.h src/controllers/StatsController.cpp
    src/controllers/RtaController.h src/controllers/RtaController.cpp
    src/render/RtaSurface.h src/render/RtaSurface.cpp
    src/render/RtaBars.h src/render/RtaBars.cpp)
list(APPEND DSPI_QML_FILES
    qml/tools/StatsFillRow.qml   # -- WP LAY0 -- StatsWindow.qml is StatsPanel.qml (lay0.cmake)
    qml/tools/SpectrumPanel.qml qml/main/SpectrumStrip.qml qml/main/StripColumnsTile.qml   # -- WP LAY1b -- was SpectrumWindow.qml
    qml/graph/RtaGraphOverlay.qml qml/graph/RtaGraphOptions.qml qml/graph/RtaChip.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeAnalysis.cpp)
