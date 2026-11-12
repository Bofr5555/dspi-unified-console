# -- WP QP4 -- On-graph PEQ editor
list(APPEND DSPI_APP_SOURCES
    src/controllers/PeqEditorController.h src/controllers/PeqEditorController.cpp
    src/render/PeqOverlay.h src/render/PeqOverlay.cpp)
list(APPEND DSPI_QML_FILES
    qml/graph/PeqGraphOverlay.qml qml/graph/BandChip.qml qml/graph/ShapeChooser.qml
    qml/graph/PeqSelection.qml)
list(APPEND DSPI_QML_SINGLETONS qml/graph/PeqSelection.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokePeq.cpp)
